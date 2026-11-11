#pragma once
#include <QHash>
#include <QVariantList>
#include <QVector>

// Rolling quota consumption, not percentage used since a window began.
// Histories never mix providers, accounts, quota windows, or reset periods.
class BurnRate {
public:
    double observe(const QString &provider, const QString &account, const QVariantList &limits,
                   qint64 fetchedAt, qint64 now, bool usable);
    void clear() { m_providers.clear(); }
private:
    struct Sample { qint64 at; double used; };
    struct Window { QVector<Sample> samples; };
    struct Provider { QString account; QHash<QString, Window> windows; };
    QHash<QString, Provider> m_providers;
};
