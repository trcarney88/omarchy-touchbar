#pragma once
#include <QObject>
#include <QPointF>
#include <QVariantList>

// Hardware and tests share stable hit regions, independent of burn-in text shifts.
class Controls : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool functionsMode READ functionsMode NOTIFY changed)
    Q_PROPERTY(int pressedKey READ pressedKey NOTIFY changed)
public:
    explicit Controls(QObject *parent = nullptr) : QObject(parent) {}
    QString app() const { return m_app; }
    bool menuMode() const { return m_app == "menu"; }
    bool aiMode() const { return m_app == "ai"; }
    bool functionsMode() const { return m_app == "functions"; }
    bool volumeMode() const { return m_app == "volume"; }
    static QVariantList apps();
    void openApp(const QString &id);
    void back() { openApp("menu"); }
    int pressedKey() const { return m_pressedKey; }
    bool providerPicker() const { return m_providerPicker; }
    void setProviders(const QVariantList &providers) { m_providers = providers; }
    void toggleProviders();
    void setSleeping(bool sleeping) { if (sleeping) cancel(); m_sleeping = sleeping; }
    void setVolumeAvailable(bool available) { if (!available && m_dragVolume) cancel(); m_volumeAvailable = available; }
    Q_INVOKABLE void begin(double x, double y);
    Q_INVOKABLE void move(double x, double y);
    Q_INVOKABLE void end();
    void cancel();
    static int keyAt(double x, double y);
signals:
    void changed();
    void activity();
    void functionKey(int number, bool pressed);
    void providerSelected(int index);
    void volumeRequested(double fraction);
    void muteRequested();
private:
    QString m_app = "menu";
    bool m_sleeping = false, m_contact = false;
    int m_pressedKey = 0;
    bool m_providerPicker = false;
    bool m_volumeAvailable = false, m_dragVolume = false;
    QString m_action;
    QVariantList m_providers;
    int providerAt(double x, double y) const;
    QString actionAt(double x, double y) const;
    static double volumeAt(double x);
};
