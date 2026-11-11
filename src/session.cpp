#include "session.h"
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusReply>
#include <QDBusObjectPath>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QDebug>
#include <unistd.h>

static const QString service = QStringLiteral("org.freedesktop.login1");
static const QString managerPath = QStringLiteral("/org/freedesktop/login1");
static const QString managerInterface = QStringLiteral("org.freedesktop.login1.Manager");
static const QString propertiesInterface = QStringLiteral("org.freedesktop.DBus.Properties");

bool Session::permits(const QVariantMap &p, bool sleeping, bool unlocked) {
    return !sleeping && unlocked && p.value("Active").toBool()
        && p.contains("LockedHint") && !p.value("LockedHint").toBool()
        && p.value("State").toString() == "active"
        && p.value("Class").toString() == "user"
        && p.value("Type").toString() == "wayland"
        && p.value("User").isValid();
}

Session::Session(QObject *parent) : QObject(parent) {
    auto bus = QDBusConnection::systemBus();
    bus.connect(service, managerPath, managerInterface, "PrepareForSleep", this, SLOT(prepareForSleep(bool)));
    // This process runs in the user manager, so its PID is not in the desktop
    // session. UWSM imports the real session ID into the user manager environment.
    QDBusInterface manager(service, managerPath, managerInterface, bus);
    manager.setTimeout(1000);
    QDBusReply<QDBusObjectPath> session = qEnvironmentVariableIsEmpty("XDG_SESSION_ID")
        ? manager.call("GetSessionByPID", uint(0))
        : manager.call("GetSession", qEnvironmentVariable("XDG_SESSION_ID"));
    if (session.isValid()) {
        m_path = session.value().path();
        bus.connect(service, m_path, propertiesInterface, "PropertiesChanged", this,
                    SLOT(propertiesChanged(QString,QVariantMap,QStringList)));
    } else qWarning() << "Cannot identify desktop session; keeping Touch Bar off:" << session.error().message();
    inhibit();
    connect(&m_check, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        const auto object = QJsonDocument::fromJson(m_check.readAllStandardOutput()).object();
        m_unlocked = code == 0 && status == QProcess::NormalExit
            && object.value("locked").isBool() && !object.value("locked").toBool();
        update();
    });
    connect(&m_check, &QProcess::errorOccurred, this, [this] { m_unlocked = false; update(); });
    connect(&m_monitors, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        const auto document = QJsonDocument::fromJson(m_monitors.readAllStandardOutput());
        m_displaysOn = false;
        if (code == 0 && status == QProcess::NormalExit && document.isArray()) {
            for (const auto &value : document.array()) {
                const auto monitor = value.toObject();
                if (monitor.value("dpmsStatus").toBool() && !monitor.value("disabled").toBool()) m_displaysOn = true;
            }
        }
        update();
    });
    connect(&m_monitors, &QProcess::errorOccurred, this, [this] { m_displaysOn = false; update(); });
    connect(&m_timer, &QTimer::timeout, this, &Session::refresh);
    m_timer.start(1000);
    refresh();
}

void Session::inhibit() {
    QDBusInterface manager(service, managerPath, managerInterface, QDBusConnection::systemBus());
    manager.setTimeout(1000);
    QDBusReply<QDBusUnixFileDescriptor> reply = manager.call("Inhibit", "sleep", "Touchmeter",
        "Power off the Touch Bar before suspend", "delay");
    m_inhibitor = reply.isValid() ? reply.value() : QDBusUnixFileDescriptor();
    if (!reply.isValid()) qWarning() << "Cannot obtain sleep delay; keeping Touch Bar off:" << reply.error().message();
}

void Session::prepareForSleep(bool sleeping) {
    m_sleeping = sleeping;
    m_unlocked = false;
    m_displaysOn = false;
    update(); // Synchronously power off before releasing the sleep delay FD.
    if (sleeping) m_inhibitor = QDBusUnixFileDescriptor();
    else { inhibit(); refresh(); }
}

void Session::propertiesChanged(const QString &, const QVariantMap &, const QStringList &) { refresh(); }

void Session::refresh() {
    m_properties.clear();
    auto bus = QDBusConnection::systemBus();
    if (!m_path.isEmpty()) {
        QDBusInterface properties(service, m_path, propertiesInterface, bus);
        properties.setTimeout(1000);
        QDBusReply<QVariantMap> reply = properties.call("GetAll", "org.freedesktop.login1.Session");
        if (reply.isValid()) m_properties = reply.value();
    }
    QDBusInterface properties(service, managerPath, propertiesInterface, bus);
    properties.setTimeout(1000);
    QDBusReply<QVariantMap> reply = properties.call("GetAll", managerInterface);
    m_sleeping = !reply.isValid() || reply.value().value("PreparingForSleep", true).toBool()
        || reply.value().value("PreparingForShutdown", true).toBool();
    if (m_check.state() == QProcess::NotRunning) m_check.start("hyprctl", {"-j", "locked"});
    else { m_unlocked = false; m_check.kill(); }
    if (m_monitors.state() == QProcess::NotRunning) m_monitors.start("hyprctl", {"-j", "monitors"});
    else { m_displaysOn = false; m_monitors.kill(); }
    update();
}

void Session::update() {
    const bool allowed = m_inhibitor.isValid() && m_displaysOn && permits(m_properties, m_sleeping, m_unlocked);
    if (allowed == m_allowed) return;
    m_allowed = allowed;
    qInfo() << "Desktop session permits Touch Bar:" << allowed;
    emit changed(allowed);
}
