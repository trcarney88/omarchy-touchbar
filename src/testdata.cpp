#include "testdata.h"
#include "backend.h"
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSaveFile>
#include <QLockFile>
#include <QTextStream>
#include <QDebug>
#include <cmath>
#include <unistd.h>

static constexpr int MaxFixture = 65536;
QStringList TestData::providerIds() {
    return {"codex", "claude", "grok", "kimi", "deepseek", "minimax", "zai", "openrouter", "synthetic", "ollama", "opencode", "cursor"};
}
QString TestData::runtimePath(bool simulator) {
    const auto runtime = qEnvironmentVariable("XDG_RUNTIME_DIR");
    return runtime.isEmpty() ? QString() : runtime + (simulator ? "/touchmeter-simulator-test.json" : "/touchmeter-test.json");
}
bool TestData::validate(const QJsonObject &data, QString *error) {
    error->clear();
    auto fail = [&](const QString &message) { *error = message; return false; };
    if (QJsonDocument(data).toJson(QJsonDocument::Compact).size() > MaxFixture) return fail("Fixture exceeds 64 KiB");
    for (auto it = data.begin(); it != data.end(); ++it)
        if (!QStringList{"version", "selected", "mode", "now", "providers", "volume"}.contains(it.key())) return fail("Unknown fixture field: " + it.key());
    if (!data.value("version").isDouble() || data.value("version").toDouble() != 1) return fail("Fixture version must be 1");
    if (!data.value("providers").isObject() || data.value("providers").toObject().isEmpty()) return fail("Fixture needs a providers object");
    const auto ids = providerIds();
    if (!ids.contains(data.value("selected").toString())) return fail("Unknown selected provider");
    if (!QStringList{"menu", "usage", "providers", "functions", "volume"}.contains(data.value("mode").toString())) return fail("Mode must be menu, usage, providers, functions, or volume");
    if (data.contains("volume")) {
        if (!data.value("volume").isObject()) return fail("volume must be an object");
        const auto volume = data.value("volume").toObject();
        for (auto it = volume.begin(); it != volume.end(); ++it)
            if (!QStringList{"percent", "muted", "available"}.contains(it.key())) return fail("Unknown volume field: " + it.key());
        if (volume.contains("percent") && (!volume.value("percent").isDouble() || volume.value("percent").toDouble() < 0 || volume.value("percent").toDouble() > 100)) return fail("Volume percent must be 0–100");
        for (const auto &key : {"muted", "available"}) if (volume.contains(key) && !volume.value(key).isBool()) return fail(QString(key) + " must be boolean");
    }
    const auto now = data.contains("now") ? QDateTime::fromString(data.value("now").toString(), Qt::ISODate) : QDateTime::currentDateTimeUtc();
    if (!now.isValid()) return fail("Invalid fixture clock (now)");
    const auto providers = data.value("providers").toObject();
    if (!providers.contains(data.value("selected").toString())) return fail("Selected provider is missing from fixture");
    for (auto it = providers.begin(); it != providers.end(); ++it) {
        if (!ids.contains(it.key()) || !it.value().isObject()) return fail("Unknown/invalid provider: " + it.key());
        const auto p = it.value().toObject();
        for (auto field = p.begin(); field != p.end(); ++field)
            if (!QStringList{"enabled", "ready", "limits", "limitsStale", "limitsFetchedAt", "usageStatusText", "burnMinutes"}.contains(field.key())) return fail("Unknown provider field: " + field.key());
        if (p.contains("burnMinutes") && (!p.value("burnMinutes").isDouble() || p.value("burnMinutes").toDouble() < 0 || p.value("burnMinutes").toDouble() > 1440)) return fail("burnMinutes must be 0–1440");
        if (!p.value("enabled").isBool() || !p.value("ready").isBool()) return fail("enabled and ready must be booleans");
        if (!p.value("limits").isArray() || p.value("limits").toArray().size() > 8) return fail("limits must be an array of at most eight windows");
        if (p.contains("limitsStale") && !p.value("limitsStale").isBool()) return fail("limitsStale must be boolean");
        if (p.contains("limitsFetchedAt") && (!p.value("limitsFetchedAt").isDouble() || p.value("limitsFetchedAt").toDouble() < 0)) return fail("Invalid limitsFetchedAt");
        if (p.contains("usageStatusText") && (!p.value("usageStatusText").isString() || p.value("usageStatusText").toString().size() > 120)) return fail("Invalid usageStatusText");
        for (const auto &limit : p.value("limits").toArray()) {
            QString limitError;
            const QJsonObject probe{{"ready", true}, {"limits", QJsonArray{limit}}};
            if (Backend::parseLimits(QJsonDocument(probe).toJson(), now, &limitError).size() != 1) return fail("Invalid limit for " + it.key() + ": " + limitError);
        }
    }
    return true;
}
static QJsonObject decodeFixture(const QByteArray &bytes, QString *error) {
    if (bytes.size() > MaxFixture) { *error = "Fixture exceeds 64 KiB"; return {}; }
    QJsonParseError parse;
    const auto doc = QJsonDocument::fromJson(bytes, &parse);
    if (parse.error || !doc.isObject()) { *error = "Fixture must be a JSON object"; return {}; }
    if (!TestData::validate(doc.object(), error)) return {};
    return doc.object();
}
static QJsonObject readFixture(const QString &path, QString *error) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) { *error = "Cannot read fixture: " + file.errorString(); return {}; }
    return decodeFixture(file.read(MaxFixture + 1), error);
}
TestData::TestData(const QString &path, bool watch, QObject *parent) : QObject(parent), m_path(path) {
    reload();
    if (watch && !path.isEmpty()) {
        m_poll.setInterval(500);
        connect(&m_poll, &QTimer::timeout, this, &TestData::reload);
        m_poll.start();
    }
}
void TestData::reload() {
    if (m_path.isEmpty()) return;
    QFile file(m_path);
    QByteArray bytes;
    QString error;
    QJsonObject data;
    if (QFileInfo(m_path).isSymLink()) error = "Test fixture must not be a symlink";
    else if (QFileInfo::exists(m_path)) {
        if (!file.open(QIODevice::ReadOnly)) error = "Cannot read test fixture";
        else bytes = file.read(MaxFixture + 1);
        if (error.isEmpty() && bytes == m_bytes && !bytes.isEmpty()) return;
        if (error.isEmpty()) data = decodeFixture(bytes, &error);
    }
    if (!error.isEmpty() && error != m_error) qWarning().noquote() << error << "— clearing fixture override";
    m_bytes = bytes; m_error = error;
    if (data != m_current) { m_current = data; emit changed(m_current); }
}

QJsonObject TestData::scenario(const QString &name, QString *error) {
    error->clear();
    if (!QStringList{"all-providers", "high-usage", "stale", "missing", "signed-out", "fire", "menu", "volume", "volume-muted", "volume-unavailable"}.contains(name)) {
        *error = "Unknown scenario. Choose all-providers, high-usage, stale, missing, signed-out, fire, menu, volume, volume-muted, or volume-unavailable"; return {};
    }
    const auto now = QDateTime::currentDateTimeUtc();
    QJsonObject providers;
    int index = 0;
    for (const auto &id : TestData::providerIds()) {
        const double shortUsage = name == "high-usage" ? 0.96 : 0.12 + index * 0.06;
        QJsonArray limits{
            QJsonObject{{"label", "5h window"}, {"percent", shortUsage}, {"resetsAt", now.addSecs(7200).toString(Qt::ISODate)}},
            QJsonObject{{"label", "Weekly (7-day)"}, {"percent", 0.61}, {"resetsAt", now.addDays(4).toString(Qt::ISODate)}}};
        if (name == "missing") limits = {};
        providers[id] = QJsonObject{{"enabled", name != "signed-out"}, {"ready", name != "signed-out"}, {"limits", limits},
            {"limitsFetchedAt", double(now.toMSecsSinceEpoch())}, {"limitsStale", name == "stale"},
            {"usageStatusText", name == "missing" ? "Simulated unavailable limits" : ""}};
        if (name == "fire") { auto p = providers[id].toObject(); p["burnMinutes"] = 20; providers[id] = p; }
        ++index;
    }
    QJsonObject data{{"version", 1}, {"selected", "claude"}, {"mode", name == "all-providers" ? "providers" : name == "menu" ? "menu" : name.startsWith("volume") ? "volume" : "usage"}, {"providers", providers}};
    if (name.startsWith("volume")) data["volume"] = QJsonObject{{"percent", 65}, {"muted", name == "volume-muted"}, {"available", name != "volume-unavailable"}};
    return data;
}
int TestData::runCli(const QStringList &input) {
    QTextStream out(stdout), err(stderr);
    auto args = input;
    const int simulatorFlags = args.removeAll("--simulator");
    if (simulatorFlags > 1) { err << "Use --simulator only once\n"; return 1; }
    const bool simulator = simulatorFlags == 1;
    const auto command = args.value(0);
    if (command.isEmpty() || command == "--help" || command == "help") {
        out << "Usage:\n"
            << "  touchmeter test scenario <all-providers|high-usage|stale|missing|signed-out|fire|menu|volume|volume-muted|volume-unavailable>\n"
            << "  touchmeter test load <fixture.json>\n"
            << "  touchmeter test set <provider> [--enabled|--disabled] [--short 0..100] [--weekly 0..100] [--burn-minutes 0..1440|--no-burn] [--stale|--fresh] [--mode menu|usage|providers|functions|volume]\n"
            << "  touchmeter test status\n  touchmeter test clear\n"
            << "Add --simulator anywhere to target simulator windows instead of the physical bar.\n"
            << "Export status JSON to edit/reload it. Preview with: touchmeter preview out.png --fixture fixture.json\n"
            << "Test state is session-local. Clear it to resume live data. Lock/suspend protections remain active.\n";
        return 0;
    }
    const auto path = runtimePath(simulator);
    if (path.isEmpty() || !QFileInfo(QFileInfo(path).absolutePath()).isDir() || QFileInfo(QFileInfo(path).absolutePath()).ownerId() != uint(getuid())) {
        err << "A user-owned XDG_RUNTIME_DIR is required\n"; return 1;
    }
    if (QFileInfo(path).isSymLink()) { err << "Refusing symlink at test state path\n"; return 1; }
    QLockFile lock(path + ".lock");
    if (command != "status" && !lock.tryLock(1000)) { err << "Test state is being changed by another process\n"; return 1; }
    QString error;
    QJsonObject data;
    if (command == "clear" && args.size() == 1) {
        if (QFileInfo::exists(path) && !QFile::remove(path)) { err << "Cannot clear test state\n"; return 1; }
        out << (simulator ? "Simulator overrides cleared; launch scene restored" : "Live mode restored") << " (running app updates within 0.5s)\n"; return 0;
    } else if (command == "status" && args.size() == 1) {
        if (!QFileInfo::exists(path)) { out << "{\"active\":false}\n"; return 0; }
        data = readFixture(path, &error);
    } else if (command == "scenario" && args.size() == 2) data = scenario(args[1], &error);
    else if (command == "load" && args.size() == 2) data = readFixture(args[1], &error);
    else if (command == "set" && args.size() >= 2) {
        if (!providerIds().contains(args[1])) { err << "Unknown provider\n"; return 1; }
        data = QFileInfo::exists(path) ? readFixture(path, &error) : scenario("signed-out", &error);
        if (error.isEmpty()) {
            auto providers = data.value("providers").toObject();
            auto p = providers.value(args[1]).toObject();
            if (p.isEmpty()) p = scenario("signed-out", &error).value("providers").toObject().value(args[1]).toObject();
            auto limits = p.value("limits").toArray();
            const auto now = data.contains("now") ? QDateTime::fromString(data.value("now").toString(), Qt::ISODate) : QDateTime::currentDateTimeUtc();
            for (int i = 2; i < args.size(); ++i) {
                const auto option = args[i];
                if (option == "--enabled" || option == "--disabled") { p["enabled"] = option == "--enabled"; p["ready"] = option == "--enabled"; }
                else if (option == "--stale" || option == "--fresh") p["limitsStale"] = option == "--stale";
                else if (option == "--no-burn") p.remove("burnMinutes");
                else if (option == "--burn-minutes" && i + 1 < args.size()) {
                    bool ok; const double minutes = args[++i].toDouble(&ok);
                    if (!ok || !std::isfinite(minutes) || minutes < 0 || minutes > 1440) { error = "Burn minutes must be 0–1440"; break; }
                    p["burnMinutes"] = minutes;
                }
                else if (option == "--mode" && i + 1 < args.size()) data["mode"] = args[++i];
                else if ((option == "--short" || option == "--weekly") && i + 1 < args.size()) {
                    bool ok; const double percentage = args[++i].toDouble(&ok);
                    if (!ok || !std::isfinite(percentage) || percentage < 0 || percentage > 100) { error = "Usage must be a percentage from 0 to 100"; break; }
                    const auto label = option == "--short" ? "5h window" : "Weekly (7-day)";
                    const auto reset = now.addSecs(option == "--short" ? 7200 : 4 * 86400).toString(Qt::ISODate);
                    QJsonObject limit{{"label", label}, {"percent", percentage / 100}, {"resetsAt", reset}};
                    bool found = false;
                    for (int j = 0; j < limits.size(); ++j) if (limits[j].toObject().value("label").toString() == label) { limits[j] = limit; found = true; break; }
                    if (!found) limits.append(limit);
                } else { error = "Unknown/incomplete option: " + option; break; }
            }
            p["limits"] = limits; p["limitsFetchedAt"] = double(now.toMSecsSinceEpoch());
            providers[args[1]] = p; data["providers"] = providers; data["selected"] = args[1];
            if (!args.contains("--mode")) data["mode"] = "usage";
        }
    } else error = "Unknown command/arguments. Use touchmeter test --help";
    if (error.isEmpty() && simulator && command != "status" && command != "load" && !data.contains("now"))
        data["now"] = QDateTime::currentDateTimeUtc().toString(Qt::ISODate);
    if (error.isEmpty()) validate(data, &error);
    if (!error.isEmpty()) { err << error << '\n'; return 1; }
    const auto bytes = QJsonDocument(data).toJson(QJsonDocument::Indented);
    if (command == "status") { out << bytes; return 0; }
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly) || !file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner)
        || file.write(bytes) != bytes.size() || !file.commit()) { err << "Cannot atomically write test state\n"; return 1; }
    out << (simulator ? "Simulator TEST override enabled" : "Physical Touch Bar TEST mode enabled")
        << " (running app updates within 0.5s). Restore with: touchmeter test clear" << (simulator ? " --simulator" : "") << '\n';
    return 0;
}
