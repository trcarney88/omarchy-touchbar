#pragma once
#include <QObject>
#include <QTimer>
#include <QJsonObject>

// Persistent preferences, independent of session-only test fixtures.
class AppConfig : public QObject {
    Q_OBJECT
public:
    explicit AppConfig(const QString &path = {}, bool watch = true, QObject *parent = nullptr);
    int fireTriggerMinutes() const { return m_fireTriggerMinutes; }
    static QString configPath();
    static bool validate(const QJsonObject &data, int *minutes, QString *error);
    static int runCli(const QStringList &args);
signals:
    void changed();
private:
    void reload();
    QString m_path, m_error;
    int m_fireTriggerMinutes = 30;
    QTimer m_poll;
};
