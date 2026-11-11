#include "backend.h"
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonArray>
#include <QJsonObject>
#include <QLocale>
#include <cmath>

static constexpr auto Demo = R"json({"ready":true,"limits":[{"label":"5h window","percent":0.23,"resetsAt":"2099-01-01T12:00:00Z"},{"label":"Weekly (7-day)","percent":0.61,"resetsAt":"2099-01-05T12:00:00Z"}]})json";

QVariantList Backend::parseLimits(const QByteArray &json, const QDateTime &now, QString *error) {
    error->clear();
    if (json.size() > 1024 * 1024) { *error = tr("Usage data is too large"); return {}; }
    QJsonParseError parseError;
    const auto doc = QJsonDocument::fromJson(json, &parseError);
    if (parseError.error || !doc.isObject()) { *error = tr("Usage data unavailable"); return {}; }
    auto root = doc.object();
    if (!root.value("ready").toBool()) { *error = tr("Usage data unavailable"); return {}; }
    const auto limits = root.value("limits").toArray();
    if (limits.isEmpty() || limits.size() > 8) { *error = tr("Limits unavailable"); return {}; }
    QVariantList result;
    for (const auto &value : limits) {
        const auto limit = value.toObject();
        const auto label = limit.value("label").toString();
        // Collector labels remain bounded plain text; never interpret them as markup.
        const bool shortTerm = label == "5h window" || label == "Session (5-hour)";
        // Collector labels are plain text, bounded, and never interpreted as markup.
        if (label.isEmpty() || label.size() > 48 || label.contains('<') || label.contains('>')) continue;
        if (!limit.value("percent").isDouble() || !limit.value("resetsAt").isString()) continue;
        const double fraction = limit.value("percent").toDouble();
        const auto reset = QDateTime::fromString(limit.value("resetsAt").toString(), Qt::ISODate);
        if (!std::isfinite(fraction) || fraction < 0 || fraction > 1 || (!reset.isValid() && !limit.value("resetsAt").toString().isEmpty()) || (reset.isValid() && now.daysTo(reset) > 366)) continue;
        const qint64 minutes = qMax<qint64>(0, (now.secsTo(reset) + 59) / 60);
        QString countdown;
        if (!reset.isValid()) countdown = tr("reset unknown");
        else if (minutes == 0) countdown = tr("reset due");
        else if (minutes >= 1440) countdown = tr("%1d %2h").arg(minutes / 1440).arg((minutes % 1440) / 60);
        else countdown = tr("%1h %2m").arg(minutes / 60).arg(minutes % 60);
        result.append(QVariantMap{{"label", shortTerm ? tr("5 HOUR") : label == "Weekly (7-day)" ? tr("WEEKLY") : label},
                                  {"window", label}, {"resetsAt", reset.isValid() ? reset.toUTC().toString(Qt::ISODate) : QString()},
                                  {"fraction", fraction}, {"countdown", countdown}, {"expired", reset.isValid() && minutes == 0}});
    }
    if (result.isEmpty()) *error = tr("Limits unavailable");
    return result;
}

Backend::Backend(const QString &path, bool demo, bool collect, QObject *parent, const QJsonObject &testData) : QObject(parent), m_path(path), m_demo(demo) {
    const QList<QPair<QString, QString>> catalog{{"codex", "OpenAI / Codex"}, {"claude", "Anthropic / Claude"},
        {"grok", "xAI / Grok"}, {"kimi", "Kimi"}, {"deepseek", "DeepSeek"}, {"minimax", "MiniMax"},
        {"zai", "Z.ai"}, {"openrouter", "OpenRouter"}, {"synthetic", "Synthetic"}, {"ollama", "Ollama Cloud"},
        {"opencode", "OpenCode Zen"}, {"cursor", "Cursor"}};
    for (const auto &p : catalog) m_providers.append(QVariantMap{{"id", p.first}, {"name", p.second}, {"enabled", demo && (p.first == "codex" || p.first == "claude")}});
    m_liveProviders = m_providers;
    if (m_path.isEmpty()) {
        const auto state = qEnvironmentVariable("XDG_STATE_HOME", QDir::homePath() + "/.local/state");
        m_path = state + "/omarchy/agents/usage/codex.json";
    }
    m_usageDirectory = QFileInfo(m_path).absolutePath();
    m_reload.setInterval(15000);
    connect(&m_reload, &QTimer::timeout, this, &Backend::reload);
    m_reload.start();
    m_timeout.setSingleShot(true); m_timeout.setInterval(60000);
    connect(&m_timeout, &QTimer::timeout, this, [this] { m_refreshFailed = true; m_process.kill(); reload(); });
    connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        m_timeout.stop(); m_refreshFailed = code != 0 || status != QProcess::NormalExit;
        m_process.readAllStandardOutput(); m_process.readAllStandardError(); reload();
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError) {
        m_refreshFailed = true; reload();
    });
    // Omarchy owns authentication and network refreshes; the host receives only availability booleans.
    m_process.setProgram("omarchy");
    m_process.setArguments({"agent", "usage", "update", "--limits-only", "codex"});
    m_credentials.setProgram("python3");
    QFile credentialScript(":/provider_credentials.py");
    if (credentialScript.open(QIODevice::ReadOnly)) m_credentials.setArguments({"-c", QString::fromUtf8(credentialScript.readAll())});
    m_credentialTimeout.setSingleShot(true); m_credentialTimeout.setInterval(10000);
    auto applyCredentials = [this](const QJsonObject &available) {
        for (auto &entry : m_liveProviders) {
            auto p = entry.toMap(); p["enabled"] = available.value(p["id"].toString()).toBool(); entry = p;
        }
        if (!testing()) m_providers = m_liveProviders;
        reload();
    };
    connect(&m_credentialTimeout, &QTimer::timeout, this, [this, applyCredentials] { m_credentials.kill(); applyCredentials({}); });
    connect(&m_credentials, &QProcess::finished, this, [this, applyCredentials](int code, QProcess::ExitStatus status) {
        m_credentialTimeout.stop();
        const auto bytes = m_credentials.readAllStandardOutput(); m_credentials.readAllStandardError();
        applyCredentials(code == 0 && status == QProcess::NormalExit && bytes.size() < 65536 ? QJsonDocument::fromJson(bytes).object() : QJsonObject());
    });
    connect(&m_credentials, &QProcess::errorOccurred, this, [this, applyCredentials](QProcess::ProcessError) { m_credentialTimeout.stop(); applyCredentials({}); });
    m_credentialTimer.setInterval(15000);
    connect(&m_credentialTimer, &QTimer::timeout, this, &Backend::checkCredentials);
    if (!demo && path.isEmpty()) { m_credentialTimer.start(); if (testData.isEmpty()) checkCredentials(); }
    m_update.setInterval(300000);
    connect(&m_update, &QTimer::timeout, this, &Backend::refresh);
    if (!demo && collect) { m_update.start(); QTimer::singleShot(0, this, &Backend::refresh); }
    if (!testData.isEmpty()) setTestData(testData);
    else reload();
}
void Backend::refresh() {
    if (m_demo || testing() || m_process.state() != QProcess::NotRunning) return;
    m_process.start(); m_timeout.start();
}
void Backend::setFireTriggerMinutes(int minutes) {
    if (minutes < 1 || minutes > 1440 || minutes == m_fireTriggerMinutes) return;
    m_fireTriggerMinutes = minutes;
    reload();
}
void Backend::checkCredentials() {
    if (testing()) return;
    if (m_credentials.state() == QProcess::NotRunning) { m_credentials.start(); m_credentialTimeout.start(); }
}
void Backend::setTestData(const QJsonObject &data) {
    const bool wasTesting = testing();
    if (wasTesting != !data.isEmpty()) m_burnRate.clear();
    if (!wasTesting && !data.isEmpty()) m_liveSelected = m_selected;
    m_testData = data;
    if (testing()) {
        m_timeout.stop(); m_credentialTimeout.stop();
        if (m_process.state() != QProcess::NotRunning) { m_process.kill(); m_process.waitForFinished(1000); }
        if (m_credentials.state() != QProcess::NotRunning) { m_credentials.kill(); m_credentials.waitForFinished(1000); }
        const auto providers = data.value("providers").toObject();
        for (int i = 0; i < m_providers.size(); ++i) {
            auto p = m_providers[i].toMap();
            p["enabled"] = providers.value(p["id"].toString()).toObject().value("enabled").toBool();
            m_providers[i] = p;
            if (p["id"].toString() == data.value("selected").toString()) m_selected = i;
        }
        if (!wasTesting) qWarning() << "TEST data active; credential probes and usage refreshes paused. Run touchmeter test clear to restore live data.";
    } else {
        m_providers = m_liveProviders;
        if (wasTesting) {
            m_selected = m_liveSelected;
            m_refreshFailed = false;
            if (m_credentialTimer.isActive()) checkCredentials();
            if (m_update.isActive()) QTimer::singleShot(0, this, &Backend::refresh);
            qInfo() << "Test data cleared; live data restored";
        }
    }
    const auto id = m_providers[m_selected].toMap().value("id").toString();
    m_path = m_usageDirectory + "/" + id + ".json";
    m_process.setArguments({"agent", "usage", "update", "--limits-only", id});
    reload();
}
bool Backend::selectProvider(int index) {
    if (index < 0 || index >= m_providers.size() || !m_providers[index].toMap().value("enabled").toBool()) return false;
    if (index == m_selected) return true;
    // Do not let a previous provider's refresh/failure become the next provider's state.
    m_timeout.stop();
    if (m_process.state() != QProcess::NotRunning) { m_process.kill(); m_process.waitForFinished(1000); }
    m_selected = index; m_refreshFailed = false;
    const auto id = m_providers[index].toMap().value("id").toString();
    m_path = m_usageDirectory + "/" + id + ".json";
    m_process.setArguments({"agent", "usage", "update", "--limits-only", id});
    reload();
    if (m_update.isActive()) refresh();
    return true;
}
void Backend::reload() {
    QFile file(m_path);
    QByteArray data;
    const auto id = m_providers[m_selected].toMap().value("id").toString();
    if (testing()) data = QJsonDocument(m_testData.value("providers").toObject().value(id).toObject()).toJson(QJsonDocument::Compact);
    else if (m_demo) data = Demo;
    else if (file.open(QIODevice::ReadOnly)) data = file.read(1024 * 1024 + 1);
    const auto now = testing() && m_testData.contains("now") ? QDateTime::fromString(m_testData.value("now").toString(), Qt::ISODate)
        : m_demo && !testing() ? QDateTime::fromString("2099-01-01T09:15:00Z", Qt::ISODate) : QDateTime::currentDateTimeUtc();
    m_limits = parseLimits(data, now, &m_status);
    const auto record = QJsonDocument::fromJson(data).object();
    if ((!m_demo || testing()) && !m_providers[m_selected].toMap().value("enabled").toBool()) { m_limits.clear(); m_status = tr("Sign-in required"); }
    else if (m_limits.isEmpty() && !record.value("usageStatusText").toString().isEmpty()) m_status = record.value("usageStatusText").toString().left(120);
    const auto fetched = record.value("limitsFetchedAt").toDouble();
    const bool validFetched = std::isfinite(fetched) && fetched > 0 && fetched <= double(now.toMSecsSinceEpoch()) + 60000;
    m_stale = (!m_demo || testing()) && ((!testing() && m_refreshFailed) || record.value("limitsStale").toBool() || !validFetched || now.toMSecsSinceEpoch() - fetched > 20 * 60000);
    for (const auto &limit : m_limits) if (limit.toMap().value("expired").toBool()) m_stale = true;
    QString account;
    for (const auto &entry : record.value("accounts").toArray()) {
        const auto a = entry.toObject();
        if (a.value("active").toBool()) { account = a.value("id").toString(); break; }
    }
    const bool usable = !m_stale && !m_limits.isEmpty() && m_providers[m_selected].toMap().value("enabled").toBool();
    if (testing()) {
        // Explicit simulation only: fictional usage never trains real burn-rate history.
        m_burnMinutes = usable && record.value("burnMinutes").isDouble() ? record.value("burnMinutes").toDouble() : -1;
    } else {
        m_burnMinutes = m_burnRate.observe(id, account, m_limits, validFetched ? qint64(fetched) : 0, now.toMSecsSinceEpoch(), usable && !m_demo);
    }
    if (m_status.isEmpty()) m_status = m_stale ? tr("STALE · retrying") : tr("LIVE · used");
    if (burning()) m_status = tr("~%1m to limit").arg(qMax(1, int(std::ceil(m_burnMinutes))));
    if (testing()) m_status.prepend("TEST · ");
    emit changed();
}
