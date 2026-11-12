use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct BurnWindow {
    pub window: String,
    pub fraction: f64,
    pub resets_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Copy)]
struct Sample {
    at: i64,
    used: f64,
}
#[derive(Default)]
struct Provider {
    account: String,
    windows: HashMap<String, VecDeque<Sample>>,
}
#[derive(Default)]
pub struct BurnRate {
    providers: HashMap<String, Provider>,
}

impl BurnRate {
    pub fn clear(&mut self) {
        self.providers.clear();
    }
    pub fn observe(
        &mut self,
        provider: &str,
        account: &str,
        windows: &[BurnWindow],
        fetched_at_ms: i64,
        now_ms: i64,
        usable: bool,
    ) -> Option<f64> {
        const HORIZON: i64 = 15 * 60_000;
        if !usable
            || fetched_at_ms <= 0
            || fetched_at_ms > now_ms + 60_000
            || now_ms - fetched_at_ms > HORIZON
        {
            self.providers.remove(provider);
            return None;
        }
        let p = self.providers.entry(provider.to_owned()).or_default();
        if p.account != account {
            p.windows.clear();
            p.account = account.to_owned();
        }
        let mut seen = HashSet::new();
        let mut soonest = None;
        for w in windows {
            let Some(reset) = w.resets_at else { continue };
            if reset.timestamp_millis() <= now_ms
                || !w.fraction.is_finite()
                || !(0.0..=1.0).contains(&w.fraction)
            {
                continue;
            }
            let key = format!("{}\n{}", w.window, reset.to_rfc3339());
            seen.insert(key.clone());
            let samples = p.windows.entry(key).or_default();
            if let Some(last) = samples.back() {
                if w.fraction < last.used
                    || fetched_at_ms < last.at
                    || fetched_at_ms - last.at > HORIZON
                {
                    samples.clear();
                }
            }
            if samples.back().map_or(true, |s| fetched_at_ms > s.at) {
                samples.push_back(Sample {
                    at: fetched_at_ms,
                    used: w.fraction,
                });
            }
            while samples.front().is_some_and(|s| now_ms - s.at > HORIZON) || samples.len() > 64 {
                samples.pop_front();
            }
            if samples.len() < 2 {
                continue;
            }
            let first = samples.front().unwrap();
            let last = samples.back().unwrap();
            let elapsed = last.at - first.at;
            let consumed = last.used - first.used;
            if elapsed < 60_000 || consumed <= 0.000001 {
                continue;
            }
            let minutes =
                (1.0 - last.used) * (elapsed.max(now_ms - first.at) as f64) / consumed / 60_000.0;
            if minutes * 60_000.0 >= (reset.timestamp_millis() - now_ms) as f64 {
                continue;
            }
            soonest = Some(soonest.map_or(minutes, |v: f64| v.min(minutes)));
        }
        p.windows.retain(|k, _| seen.contains(k));
        soonest
    }
}
