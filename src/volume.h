#pragma once
#include <QObject>
#include <QProcess>
#include <QTimer>
#include <QJsonObject>
#include <optional>

// Default PipeWire output only. Serialized, bounded wpctl calls; no shell and no root.
class Volume : public QObject {
    Q_OBJECT
public:
    explicit Volume(bool simulation = false, const QString &program = "wpctl", QObject *parent = nullptr);
    ~Volume();
    double fraction() const { return m_volume; }
    bool muted() const { return m_muted; }
    bool available() const { return m_available; }
    bool simulated() const { return m_simulation; }
    QString status() const { return m_available ? QString() : tr("Audio unavailable"); }
    void setActive(bool active);
    void setSimulation(bool simulation, const QJsonObject &data = {});
    void setVolume(double fraction);
    void toggleMute();
    static bool parse(const QByteArray &bytes, double *fraction, bool *muted);
signals:
    void changed();
private:
    enum class Operation { Read, Volume, Mute };
    void refresh();
    void pump();
    void finished(bool success);
    void stop();
    bool m_active = false, m_simulation = false, m_available = false, m_muted = false;
    double m_volume = 0;
    QString m_program;
    Operation m_operation = Operation::Read;
    QProcess m_process;
    QTimer m_poll, m_flush, m_timeout;
    bool m_readPending = false;
    std::optional<double> m_volumePending;
    std::optional<bool> m_mutePending;
};
