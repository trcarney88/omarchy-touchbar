#include "volume.h"
#include <QRegularExpression>
#include <QProcessEnvironment>
#include <cmath>

Volume::Volume(bool simulation, const QString &program, QObject *parent) : QObject(parent), m_program(program) {
    auto environment = QProcessEnvironment::systemEnvironment(); environment.insert("LC_ALL", "C");
    m_process.setProcessEnvironment(environment);
    m_poll.setInterval(1000);
    m_flush.setSingleShot(true); m_flush.setInterval(40);
    m_timeout.setSingleShot(true); m_timeout.setInterval(2000);
    connect(&m_poll, &QTimer::timeout, this, &Volume::refresh);
    connect(&m_flush, &QTimer::timeout, this, &Volume::pump);
    connect(&m_timeout, &QTimer::timeout, this, [this] { m_process.kill(); });
    connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus status) {
        finished(code == 0 && status == QProcess::NormalExit);
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) finished(false);
    });
    setSimulation(simulation);
}
Volume::~Volume() { m_active = false; stop(); }
void Volume::stop() {
    m_poll.stop(); m_flush.stop(); m_timeout.stop();
    m_volumePending.reset(); m_mutePending.reset(); m_readPending = false;
    if (m_process.state() != QProcess::NotRunning) {
        m_process.kill(); m_process.waitForFinished(1000);
    }
}
void Volume::setActive(bool active) {
    if (active == m_active) return;
    m_active = false; stop(); m_active = active;
    if (active && !m_simulation) {
        m_available = false; emit changed();
        m_poll.start(); refresh();
    }
}
void Volume::setSimulation(bool simulation, const QJsonObject &data) {
    const bool active = m_active;
    m_active = false; stop(); m_simulation = simulation;
    if (simulation) {
        m_volume = qBound(0.0, data.value("percent").toDouble(65) / 100.0, 1.0);
        m_muted = data.value("muted").toBool();
        m_available = data.value("available").toBool(true);
    } else { m_available = false; }
    m_active = active;
    emit changed();
    if (active && !simulation) { m_poll.start(); refresh(); }
}
bool Volume::parse(const QByteArray &bytes, double *fraction, bool *muted) {
    if (bytes.size() > 256) return false;
    static const QRegularExpression pattern("^Volume:\\s+([0-9]+(?:\\.[0-9]+)?)\\s*(\\[MUTED\\])?\\s*$");
    const auto match = pattern.match(QString::fromUtf8(bytes).trimmed());
    if (!match.hasMatch()) return false;
    bool ok; const double value = match.captured(1).toDouble(&ok);
    if (!ok || !std::isfinite(value)) return false;
    *fraction = qBound(0.0, value, 1.0); // Reflect boosted sinks safely; this app never requests boosts.
    *muted = !match.captured(2).isEmpty();
    return true;
}
void Volume::setVolume(double fraction) {
    if (!m_active || !m_available || !std::isfinite(fraction)) return;
    m_volume = qBound(0.0, fraction, 1.0);
    emit changed();
    if (!m_simulation) {
        m_volumePending = m_volume;
        // Fixed-rate coalescing, not a debounce that waits until a finger stops moving.
        if (!m_flush.isActive()) m_flush.start();
    }
}
void Volume::toggleMute() {
    if (!m_active || !m_available) return;
    m_muted = !m_muted; emit changed();
    if (!m_simulation) { m_mutePending = m_muted; if (!m_flush.isActive()) m_flush.start(); }
}
void Volume::refresh() {
    if (!m_active || m_simulation) return;
    m_readPending = true; pump();
}
void Volume::pump() {
    if (!m_active || m_simulation || m_process.state() != QProcess::NotRunning || m_flush.isActive()) return;
    QStringList args;
    if (m_volumePending) {
        m_operation = Operation::Volume;
        args = {"set-volume", "--limit", "1.0", "@DEFAULT_AUDIO_SINK@", QString::number(*m_volumePending, 'f', 3)};
        m_volumePending.reset();
    } else if (m_mutePending) {
        m_operation = Operation::Mute;
        args = {"set-mute", "@DEFAULT_AUDIO_SINK@", *m_mutePending ? "1" : "0"};
        m_mutePending.reset();
    } else if (m_readPending) {
        m_operation = Operation::Read; args = {"get-volume", "@DEFAULT_AUDIO_SINK@"}; m_readPending = false;
    } else return;
    m_process.setProgram(m_program); m_process.setArguments(args);
    m_timeout.start(); m_process.start();
}
void Volume::finished(bool success) {
    m_timeout.stop();
    const auto output = m_process.readAllStandardOutput(); m_process.readAllStandardError();
    if (!m_active || m_simulation) return;
    double value = 0; bool muted = false;
    if (success && m_operation == Operation::Read) success = parse(output, &value, &muted);
    if (!success) {
        m_available = false; m_volumePending.reset(); m_mutePending.reset(); m_readPending = false;
        m_flush.stop(); emit changed(); return; // The next poll retries the default sink.
    }
    if (m_operation == Operation::Read) {
        // A query started before a drag/tap must not overwrite newer optimistic state.
        if (!m_volumePending) m_volume = value;
        if (!m_mutePending) m_muted = muted;
        m_available = true;
    } else m_readPending = true;
    emit changed(); pump();
}
