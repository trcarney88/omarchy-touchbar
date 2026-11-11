#include "appconfig.h"
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QLockFile>
#include <QSaveFile>
#include <QTextStream>
#include <QDebug>
#include <cmath>

namespace {
constexpr int MaxConfig = 65536;
constexpr auto Key = "fire.trigger-minutes";
QJsonObject defaults() { return {{"version", 1}, {"fire", QJsonObject{{"triggerMinutes", 30}}}}; }
QJsonObject readConfig(const QString &path, QString *error) {
    if (!QFileInfo::exists(path)) return defaults();
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) { *error = "Cannot read config: " + file.errorString(); return {}; }
    const auto bytes = file.read(MaxConfig + 1);
    if (bytes.size() > MaxConfig) { *error = "Config exceeds 64 KiB"; return {}; }
    QJsonParseError parse;
    const auto doc = QJsonDocument::fromJson(bytes, &parse);
    if (parse.error || !doc.isObject()) { *error = "Config must be a JSON object"; return {}; }
    int minutes;
    if (!AppConfig::validate(doc.object(), &minutes, error)) return {};
    return doc.object();
}
}

QString AppConfig::configPath() {
    auto home = qEnvironmentVariable("XDG_CONFIG_HOME");
    if (home.isEmpty() || !QDir::isAbsolutePath(home)) home = QDir::homePath() + "/.config";
    return home + "/touchmeter/config.json";
}
bool AppConfig::validate(const QJsonObject &data, int *minutes, QString *error) {
    error->clear(); *minutes = 30;
    auto fail = [&](const QString &message) { *error = message; return false; };
    if (!data.value("version").isDouble() || data.value("version").toDouble() != 1) return fail("Config version must be 1");
    for (auto it = data.begin(); it != data.end(); ++it)
        if (it.key() != "version" && it.key() != "fire") return fail("Unknown config section: " + it.key());
    if (!data.contains("fire")) return true;
    if (!data.value("fire").isObject()) return fail("fire must be an object");
    const auto fire = data.value("fire").toObject();
    for (auto it = fire.begin(); it != fire.end(); ++it)
        if (it.key() != "triggerMinutes") return fail("Unknown fire setting: " + it.key());
    if (!fire.contains("triggerMinutes")) return true;
    const auto value = fire.value("triggerMinutes");
    const auto number = value.toDouble(-1);
    if (!value.isDouble() || !std::isfinite(number) || number < 1 || number > 1440 || number != std::floor(number))
        return fail("fire.trigger-minutes must be an integer from 1 to 1440");
    *minutes = int(number);
    return true;
}
AppConfig::AppConfig(const QString &path, bool watch, QObject *parent)
    : QObject(parent), m_path(path.isEmpty() ? configPath() : path) {
    reload();
    if (watch) {
        m_poll.setInterval(500);
        connect(&m_poll, &QTimer::timeout, this, &AppConfig::reload);
        m_poll.start();
    }
}
void AppConfig::reload() {
    QString error;
    const auto data = readConfig(m_path, &error);
    int minutes = 30;
    if (error.isEmpty()) validate(data, &minutes, &error);
    if (!error.isEmpty()) {
        if (error != m_error) qWarning().noquote() << error << "— keeping last valid configuration";
        m_error = error; return;
    }
    m_error.clear();
    if (minutes != m_fireTriggerMinutes) { m_fireTriggerMinutes = minutes; emit changed(); }
}
int AppConfig::runCli(const QStringList &args) {
    QTextStream out(stdout), err(stderr);
    const auto command = args.value(0);
    if (command.isEmpty() || command == "help" || command == "--help") {
        out << "Usage:\n  touchmeter config show\n  touchmeter config get <key>\n"
            << "  touchmeter config set <key> <value>\n  touchmeter config reset [key]\n  touchmeter config path\n\n"
            << "Settings:\n  fire.trigger-minutes   Projected minutes to quota exhaustion that trigger flames\n"
            << "                         Integer 1–1440; default 30. Larger = earlier warning.\n"
            << "Changes persist and apply live, including in test mode. No restart or administrator access needed.\n";
        return 0;
    }
    if (command == "path" && args.size() == 1) { out << configPath() << '\n'; return 0; }
    const bool show = command == "show" && args.size() == 1;
    const bool get = command == "get" && args.size() == 2;
    const bool set = command == "set" && args.size() == 3;
    const bool reset = command == "reset" && (args.size() == 1 || args.size() == 2);
    if (!show && !get && !set && !reset) { err << "Unknown command/arguments. Use touchmeter config --help\n"; return 1; }
    if ((get || set || (reset && args.size() == 2)) && args[1] != Key) { err << "Unknown setting: " << args[1] << '\n'; return 1; }
    int requested = 30;
    if (set) {
        bool ok; const double value = args[2].toDouble(&ok);
        if (!ok || !std::isfinite(value) || value < 1 || value > 1440 || value != std::floor(value)) {
            err << "fire.trigger-minutes must be an integer from 1 to 1440\n"; return 1;
        }
        requested = int(value);
    }
    const auto path = configPath();
    QString error;
    if (show || get) {
        const auto data = readConfig(path, &error);
        int minutes;
        if (!error.isEmpty() || !validate(data, &minutes, &error)) { err << error << '\n'; return 1; }
        if (get) out << minutes << '\n';
        else out << QJsonDocument(QJsonObject{{Key, minutes}}).toJson(QJsonDocument::Indented);
        return 0;
    }
    const auto directory = QFileInfo(path).absolutePath();
    if (!QDir().mkpath(directory)) { err << "Cannot create config directory\n"; return 1; }
    QLockFile lock(path + ".lock");
    if (!lock.tryLock(1000)) { err << "Config is being changed by another process\n"; return 1; }
    // QSaveFile follows file symlinks. Never let a config write overwrite another target.
    if (QFileInfo(path).isSymLink()) { err << "Refusing to overwrite a symlinked config\n"; return 1; }
    auto data = readConfig(path, &error);
    int oldMinutes;
    if (!error.isEmpty() || !validate(data, &oldMinutes, &error)) { err << error << "; config left unchanged\n"; return 1; }
    auto fire = data.value("fire").toObject();
    fire["triggerMinutes"] = requested; data["fire"] = fire;
    int minutes;
    if (!validate(data, &minutes, &error)) { err << error << '\n'; return 1; }
    if (QFileInfo::exists(path)) {
        const auto backup = path + ".bak." + QDateTime::currentDateTimeUtc().toString("yyyyMMddTHHmmsszzz") + "." + QString::number(QCoreApplication::applicationPid());
        if (!QFile::copy(path, backup)) { err << "Cannot back up existing config; unchanged\n"; return 1; }
    }
    QSaveFile file(path);
    const auto bytes = QJsonDocument(data).toJson(QJsonDocument::Indented);
    if (!file.open(QIODevice::WriteOnly) || !file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner)
        || file.write(bytes) != bytes.size() || !file.commit()) { err << "Cannot atomically save config\n"; return 1; }
    out << Key << " = " << minutes << " (saved; running app updates within 0.5s)\n";
    return 0;
}
