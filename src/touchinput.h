#pragma once
#include <QObject>
#include <QPointF>
#include <memory>

class TouchInput : public QObject {
    Q_OBJECT
public:
    explicit TouchInput(QObject *parent = nullptr);
    ~TouchInput() override;
    bool start(QString *error);
    static QPointF panelPoint(int x, int y, int xmin, int xmax, int ymin, int ymax);
signals:
    void began(double x, double y);
    void moved(double x, double y);
    void ended();
    void cancelled();
    void failed(const QString &error);
private:
    struct State;
    std::unique_ptr<State> s;
    void readEvents();
    void report();
};
