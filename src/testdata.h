#pragma once
#include <QObject>
#include <QJsonObject>
#include <QTimer>

// Per-session fixtures never touch real usage records, authentication, or power policy.
class TestData : public QObject {
    Q_OBJECT
public:
    explicit TestData(const QString &path, bool watch, QObject *parent = nullptr);
    QJsonObject current() const { return m_current; }
    QString error() const { return m_error; }
    static QString runtimePath(bool simulator = false);
    static QStringList providerIds();
    static bool validate(const QJsonObject &, QString *error);
    static QJsonObject scenario(const QString &name, QString *error);
    static int runCli(const QStringList &args);
signals:
    void changed(const QJsonObject &data);
private:
    void reload();
    QString m_path, m_error;
    QByteArray m_bytes;
    QJsonObject m_current;
    QTimer m_poll;
};
