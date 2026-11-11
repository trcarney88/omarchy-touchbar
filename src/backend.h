#pragma once
#include <QObject>
#include <QVariantList>
#include <QDateTime>
#include <QProcess>
#include <QTimer>
#include <QJsonObject>
#include "burnrate.h"

class Backend : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantList limits READ limits NOTIFY changed)
    Q_PROPERTY(QString status READ status NOTIFY changed)
    Q_PROPERTY(bool stale READ stale NOTIFY changed)
public:
    explicit Backend(const QString &path = {}, bool demo = false, bool collect = true, QObject *parent = nullptr, const QJsonObject &testData = {});
    QVariantList limits() const { return m_limits; }
    QString status() const { return m_status; }
    bool stale() const { return m_stale; }
    QVariantList providers() const { return m_providers; }
    int selectedProvider() const { return m_selected; }
    bool selectProvider(int index);
    void setTestData(const QJsonObject &data);
    bool testing() const { return !m_testData.isEmpty(); }
    double burnMinutes() const { return m_burnMinutes; }
    int fireTriggerMinutes() const { return m_fireTriggerMinutes; }
    void setFireTriggerMinutes(int minutes);
    bool burning() const { return m_burnMinutes >= 0 && m_burnMinutes <= m_fireTriggerMinutes; }
    Q_INVOKABLE void refresh();
    void reload();
    static QVariantList parseLimits(const QByteArray &json, const QDateTime &now, QString *error);
signals:
    void changed();
private:
    QString m_path, m_status;
    QVariantList m_limits;
    bool m_stale = true, m_demo = false, m_refreshFailed = false;
    QProcess m_process;
    QTimer m_update, m_reload, m_timeout;
    QVariantList m_providers;
    QVariantList m_liveProviders;
    QJsonObject m_testData;
    int m_liveSelected = 0;
    QString m_usageDirectory;
    int m_selected = 0;
    QProcess m_credentials;
    QTimer m_credentialTimer, m_credentialTimeout;
    void checkCredentials();
    BurnRate m_burnRate;
    double m_burnMinutes = -1;
    int m_fireTriggerMinutes = 30;
};
