#pragma once
#include <QObject>
#include <QImage>
#include <QString>
#include <memory>

// Only leases a DSI connector whose mode matches an Apple Touch Bar.
// Normal desktop displays are never requested or modeset.
class Lease : public QObject {
    Q_OBJECT
public:
    explicit Lease(QObject *parent = nullptr);
    ~Lease() override;
    bool start(QString *error);
    bool present(const QImage &image, QString *error);
    bool ready() const;
    bool setPower(bool on, QString *error);
    bool functionKey(int number, bool pressed, QString *error);
signals:
    void available();
    void failed(const QString &message);
    void idleChanged(bool idle);
private:
    struct State;
    std::unique_ptr<State> s;
};
