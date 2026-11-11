#pragma once
#include <QObject>
#include <QDBusUnixFileDescriptor>
#include <QProcess>
#include <QTimer>
#include <QVariantMap>

// Fail closed: no panel or input until both logind and Hyprland confirm access.
class Session : public QObject {
    Q_OBJECT
public:
    explicit Session(QObject *parent = nullptr);
    bool allowed() const { return m_allowed; }
    static bool permits(const QVariantMap &properties, bool sleeping, bool unlocked);
signals:
    void changed(bool allowed);
private slots:
    void refresh();
    void prepareForSleep(bool sleeping);
    void propertiesChanged(const QString &, const QVariantMap &, const QStringList &);
private:
    void update();
    void inhibit();
    QString m_path;
    QVariantMap m_properties;
    bool m_sleeping = true, m_unlocked = false, m_displaysOn = false, m_allowed = false;
    QDBusUnixFileDescriptor m_inhibitor;
    QProcess m_check;
    QProcess m_monitors;
    QTimer m_timer;
};
