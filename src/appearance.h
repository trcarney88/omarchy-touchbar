#pragma once
#include <QObject>
#include <QColor>
#include <QTimer>
#include <QTranslator>
#include <QJsonObject>
#include <QVariantMap>
#include <QProcess>

class Appearance : public QTranslator {
    Q_OBJECT
    Q_PROPERTY(QColor accent READ accent NOTIFY changed)
    Q_PROPERTY(QString language READ language NOTIFY changed)
    Q_PROPERTY(QVariantMap colors READ colors NOTIFY changed)
    Q_PROPERTY(QColor accentForeground READ accentForeground NOTIFY changed)
    Q_PROPERTY(int radius READ radius NOTIFY changed)
public:
    explicit Appearance(bool demo = false, const QString &languageOverride = {}, QObject *parent = nullptr);
    QColor accent() const { return m_accent; }
    QString language() const { return m_language; }
    QVariantMap colors() const { return m_colors; }
    QColor accentForeground() const;
    int radius() const { return m_radius; }
    static QVariantMap parseColors(const QByteArray &data);
    bool isEmpty() const override { return false; }
    QString translate(const char *, const char *source, const char * = nullptr, int = -1) const override;
signals:
    void changed();
private:
    void reload();
    QColor m_accent = QColor("#FFD60A");
    QString m_language;
    QJsonObject m_strings;
    QTimer m_poll;
    bool m_demo;
    QString m_languageOverride;
    QVariantMap m_colors;
    int m_radius = 0;
    QProcess m_rounding;
};
