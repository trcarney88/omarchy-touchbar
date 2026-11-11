#include "burnrate.h"
#include <QDateTime>
#include <QSet>
#include <cmath>
#include <limits>

double BurnRate::observe(const QString &provider, const QString &account, const QVariantList &limits,
                         qint64 fetchedAt, qint64 now, bool usable) {
    constexpr qint64 Horizon = 15 * 60000;
    if (!usable || fetchedAt <= 0 || fetchedAt > now + 60000 || now - fetchedAt > Horizon) {
        m_providers.remove(provider); return -1;
    }
    auto &p = m_providers[provider];
    if (p.account != account) { p.windows.clear(); p.account = account; }
    QSet<QString> seen;
    double soonest = std::numeric_limits<double>::infinity();
    for (const auto &value : limits) {
        const auto limit = value.toMap();
        const auto reset = QDateTime::fromString(limit.value("resetsAt").toString(), Qt::ISODate);
        const double used = limit.value("fraction").toDouble();
        if (!reset.isValid() || reset.toMSecsSinceEpoch() <= now || !std::isfinite(used) || used < 0 || used > 1) continue;
        const auto key = limit.value("window").toString() + "\n" + reset.toUTC().toString(Qt::ISODate);
        seen.insert(key);
        auto &samples = p.windows[key].samples;
        // Falling usage is a refill/account change, never negative consumption.
        if (!samples.isEmpty() && (used < samples.last().used || fetchedAt < samples.last().at
            || fetchedAt - samples.last().at > Horizon)) samples.clear();
        if (samples.isEmpty() || fetchedAt > samples.last().at) samples.append({fetchedAt, used});
        // An unchanged fetchedAt is a cached record, not another measurement.
        while (!samples.isEmpty() && (now - samples.first().at > Horizon || samples.size() > 64)) samples.removeFirst();
        if (samples.size() < 2) continue;
        const auto &first = samples.first(); const auto &last = samples.last();
        const auto elapsed = last.at - first.at;
        const double consumed = last.used - first.used;
        if (elapsed < 60000 || consumed <= 0.000001) continue;
        // Let the rate cool during quiet periods, including between collector refreshes.
        const double minutes = (1.0 - last.used) * double(qMax(elapsed, now - first.at)) / consumed / 60000.0;
        if (minutes * 60000.0 >= reset.toMSecsSinceEpoch() - now) continue; // Refill comes first.
        soonest = qMin(soonest, minutes);
    }
    for (auto it = p.windows.begin(); it != p.windows.end();) {
        if (!seen.contains(it.key())) it = p.windows.erase(it); else ++it;
    }
    return std::isfinite(soonest) ? soonest : -1;
}
