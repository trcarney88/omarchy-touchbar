#pragma once
#include <QWindow>
#include <QBackingStore>
#include <QTimer>
#include <QImage>
class Backend;
class Appearance;
class Controls;
class RuneUi;
class Volume;

// Desktop shell for the real Rune scene. It never leases a panel or injects keys.
class Simulator : public QWindow {
    Q_OBJECT
public:
    Simulator(RuneUi &, Backend &, Appearance &, Controls &, Volume &);
    ~Simulator();
    static QRectF barRect(const QSize &size);
    static QPointF panelPoint(const QPointF &point, const QSize &size);
    static QString help();
protected:
    bool event(QEvent *) override;
    void exposeEvent(QExposeEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void mousePressEvent(QMouseEvent *) override;
    void mouseMoveEvent(QMouseEvent *) override;
    void mouseReleaseEvent(QMouseEvent *) override;
    void keyPressEvent(QKeyEvent *) override;
private:
    void schedule();
    void paint();
    void cancel();
    RuneUi &m_ui;
    Backend &m_backend;
    Appearance &m_appearance;
    Controls &m_controls;
    Volume &m_volume;
    QBackingStore m_store;
    QTimer m_redraw, m_animation;
    bool m_contact = false;
    bool m_fullRepaint = true;
    QString m_title;
};
