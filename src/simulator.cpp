#include "simulator.h"
#include "runeui.h"
#include "backend.h"
#include "appearance.h"
#include "controls.h"
#include "volume.h"
#include <QMouseEvent>
#include <QKeyEvent>
#include <QPainter>
#include <QExposeEvent>
#include <QResizeEvent>
#include <QGuiApplication>
#include <QRegion>

QString Simulator::help() {
    return "Usage: touchmeter simulate [--scenario <name> | --fixture <file.json>]\n"
           "       [--menu|--ai|--functions|--providers|--volume] [--monitor <index|name>] [--fullscreen]\n"
           "       [--demo] [--language <code>]\n"
           "       touchmeter simulate --list-monitors\n\n"
           "Movable/resizable desktop window using the real Rune UI. Drag the volume slider with a mouse.\n"
           "Default: fictional all-provider demo at the menu. F keys and audio are always simulated.\n"
           "Scenarios match 'touchmeter test scenario', including fire, volume-muted and volume-unavailable.\n"
           "Change running simulator data with the same 'touchmeter test' commands plus --simulator.\n"
           "Example: touchmeter test set claude --enabled --short 95 --burn-minutes 20 --simulator\n"
           "Clear simulator overrides: touchmeter test clear --simulator (restores this window's launch scene).\n"
           "Fixtures reload live in this window only; simulator overrides win until cleared. The physical bar is untouched.\n"
           "F11: fullscreen; Escape: leave fullscreen/close; M: menu.\n"
           "On Wayland, window placement is compositor-controlled; fullscreen targets the chosen monitor.\n";
}
QRectF Simulator::barRect(const QSize &size) {
    const double availableWidth = qMax(0, size.width() - 24);
    const double availableHeight = qMax(0, size.height() - 54);
    const double scale = qMin(availableWidth / 2008.0, availableHeight / 60.0);
    const QSizeF bar(2008 * scale, 60 * scale);
    return {(size.width() - bar.width()) / 2, 12 + (availableHeight - bar.height()) / 2, bar.width(), bar.height()};
}
QPointF Simulator::panelPoint(const QPointF &point, const QSize &size) {
    const auto bar = barRect(size);
    if (bar.width() <= 0 || bar.height() <= 0) return {-1, -1};
    return {(point.x() - bar.left()) * 2008 / bar.width(), (point.y() - bar.top()) * 60 / bar.height()};
}
Simulator::Simulator(RuneUi &ui, Backend &backend, Appearance &appearance, Controls &controls, Volume &volume)
    : m_ui(ui), m_backend(backend), m_appearance(appearance), m_controls(controls), m_volume(volume), m_store(this) {
    m_title = "Touchmeter Simulator — audio and F keys simulated";
    if (!volume.simulated()) volume.setSimulation(true);
    setTitle(m_title); setMinimumSize({502, 120}); setSurfaceType(QWindow::RasterSurface);
    m_redraw.setSingleShot(true); m_redraw.setInterval(16);
    m_animation.setInterval(33);
    connect(&m_redraw, &QTimer::timeout, this, &Simulator::paint);
    connect(&m_animation, &QTimer::timeout, this, &Simulator::schedule);
    connect(&backend, &Backend::changed, this, &Simulator::schedule);
    connect(&appearance, &Appearance::changed, this, &Simulator::schedule);
    connect(&volume, &Volume::changed, this, &Simulator::schedule);
    connect(&controls, &Controls::changed, this, [this] {
        m_volume.setActive(isExposed() && windowState() != Qt::WindowMinimized && m_controls.volumeMode()); schedule();
    });
    connect(&controls, &Controls::functionKey, this, [this](int number, bool pressed) {
        setTitle(pressed ? m_title + QString(" — F%1 held").arg(number) : m_title);
    });
}
Simulator::~Simulator() { cancel(); m_redraw.stop(); m_animation.stop(); m_volume.setActive(false); }
void Simulator::cancel() { m_contact = false; m_controls.cancel(); }
void Simulator::schedule() { if (isExposed() && windowState() != Qt::WindowMinimized && !m_redraw.isActive()) m_redraw.start(); }
bool Simulator::event(QEvent *event) {
    if (event->type() == QEvent::FocusOut || event->type() == QEvent::WindowDeactivate || event->type() == QEvent::Hide || event->type() == QEvent::Close) cancel();
    if (event->type() == QEvent::Hide || event->type() == QEvent::Close) { m_animation.stop(); m_volume.setActive(false); }
    if (event->type() == QEvent::WindowStateChange && windowState() == Qt::WindowMinimized) {
        cancel(); m_animation.stop(); m_volume.setActive(false);
    }
    return QWindow::event(event);
}
void Simulator::exposeEvent(QExposeEvent *) {
    if (isExposed()) { m_fullRepaint = true; schedule(); }
    else { cancel(); m_animation.stop(); m_volume.setActive(false); }
}
void Simulator::resizeEvent(QResizeEvent *) { cancel(); m_fullRepaint = true; m_store.resize(size()); schedule(); }
void Simulator::paint() {
    if (!isExposed() || size().isEmpty() || windowState() == Qt::WindowMinimized) return;
    m_volume.setActive(m_controls.volumeMode()); // Volume is permanently simulated in this process.
    const bool animate = m_backend.burning() && m_controls.aiMode() && !m_controls.providerPicker();
    if (animate && !m_animation.isActive()) m_animation.start();
    else if (!animate) m_animation.stop();
    const auto frame = m_ui.render();
    if (frame.isNull()) { qCritical("Simulator could not render Rune"); QCoreApplication::exit(1); return; }
    // Animate only the thin bar, not a fullscreen monitor's black letterboxing.
    const QRegion dirty(m_fullRepaint ? QRect(QPoint(0, 0), size()) : barRect(size()).toAlignedRect());
    m_store.resize(size()); m_store.beginPaint(dirty);
    {
        QPainter painter(m_store.paintDevice());
        painter.setClipRegion(dirty);
        painter.fillRect(QRect(QPoint(0, 0), size()), QColor("#08090b"));
        painter.setRenderHint(QPainter::SmoothPixmapTransform);
        painter.drawImage(barRect(size()), frame);
        painter.setPen(QColor("#9da4ae"));
        painter.drawText(QRect(12, height() - 32, qMax(0, width() - 24), 24), Qt::AlignCenter,
                         "Simulated audio and F keys  ·  F11 fullscreen  ·  Esc back/close  ·  M menu");
    }
    m_store.endPaint(); m_store.flush(dirty);
    m_fullRepaint = false;
}
void Simulator::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || m_contact || !barRect(size()).contains(event->position())) return;
    m_contact = true;
    const auto point = panelPoint(event->position(), size()); m_controls.begin(point.x(), point.y());
    event->accept();
}
void Simulator::mouseMoveEvent(QMouseEvent *event) {
    if (!m_contact) return;
    if (!(event->buttons() & Qt::LeftButton)) { cancel(); return; }
    const auto point = panelPoint(event->position(), size()); m_controls.move(point.x(), point.y());
    event->accept();
}
void Simulator::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !m_contact) return;
    m_contact = false;
    const auto point = panelPoint(event->position(), size());
    m_controls.move(point.x(), point.y()); m_controls.end(); event->accept();
}
void Simulator::keyPressEvent(QKeyEvent *event) {
    if (event->isAutoRepeat()) { event->ignore(); return; }
    if (event->key() == Qt::Key_F11) {
        cancel(); setWindowState(windowState() == Qt::WindowFullScreen ? Qt::WindowNoState : Qt::WindowFullScreen);
    } else if (event->key() == Qt::Key_Escape) {
        cancel(); if (windowState() == Qt::WindowFullScreen) setWindowState(Qt::WindowNoState); else close();
    } else if (event->key() == Qt::Key_M) { cancel(); m_controls.back(); }
    else QWindow::keyPressEvent(event);
}
