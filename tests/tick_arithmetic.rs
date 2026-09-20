//! Exhaustive boundary tests for tick arithmetic.
//!
//! The oracle is exact integer arithmetic: `price = ticks * tick_units / 10^digits` is rendered
//! as a decimal string and parsed, which yields the correctly-rounded `f64` for that decimal.
//! Nothing in the oracle goes through the floating-point multiply/divide under test.

use mt5_bridge::*;

/// Exhaustive tick range: 1.5M in optimized builds (`cargo test --release`), 150k in debug so a
/// plain `cargo test` stays quick. CI should run the release sweep.
const EXHAUSTIVE: i64 = if cfg!(debug_assertions) { 150_000 } else { 1_500_000 };
const SAMPLES: usize = if cfg!(debug_assertions) { 20_000 } else { 200_000 };

/// (tick_size, digits, tick expressed as an integer number of 10^-digits units)
const CONFIGS: &[(f64, u32, i128)] = &[
    (0.00001, 5, 1),   // 5-digit FX
    (0.00005, 5, 5),
    (0.0001, 4, 1),    // 4-digit FX
    (0.001, 3, 1),     // JPY 3-digit
    (0.005, 3, 5),
    (0.01, 2, 1),      // JPY 2-digit / crypto / metals
    (0.05, 2, 5),
    (0.1, 1, 1),
    (0.25, 2, 25),     // index futures style
    (0.5, 1, 5),
    (1.0, 0, 1),
    (0.000001, 6, 1),  // 6-digit exotics
];

/// Exact decimal rendering of `n` ticks: n * tick_units / 10^digits.
fn oracle_price(n: i64, tick_units: i128, digits: u32) -> f64 {
    let units = n as i128 * tick_units; // exact integer count of 10^-digits
    let neg = units < 0;
    let a = units.abs();
    let scale = 10i128.pow(digits);
    let (int, frac) = (a / scale, a % scale);
    let s = if digits == 0 {
        format!("{}{int}", if neg { "-" } else { "" })
    } else {
        format!("{}{int}.{frac:0width$}", if neg { "-" } else { "" }, width = digits as usize)
    };
    s.parse().unwrap()
}

/// Small deterministic PRNG so the sweep is reproducible without extra dependencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }
}

#[test]
fn ticks_to_price_matches_the_exact_decimal_oracle_bit_for_bit() {
    for &(ts, d, tu) in CONFIGS {
        // exhaustive over the first 1.5M ticks (every realistic FX price and far beyond)…
        for n in 0..=EXHAUSTIVE {
            let got = ticks_to_price(n, ts, d);
            let want = oracle_price(n, tu, d);
            assert_eq!(got.to_bits(), want.to_bits(), "ts={ts} d={d} n={n}: {got} != {want}");
        }
        // …plus a large random sample up to prices of ~10^9 / negative ticks (spreads/deltas).
        let mut rng = Lcg(0x5EED ^ (d as u64) << 8 ^ tu as u64);
        for _ in 0..SAMPLES {
            let n = (rng.next() % 2_000_000_000) as i64 - 500_000_000;
            let got = ticks_to_price(n, ts, d);
            let want = oracle_price(n, tu, d);
            assert_eq!(got.to_bits(), want.to_bits(), "ts={ts} d={d} n={n}");
        }
    }
}

#[test]
fn price_to_ticks_inverts_ticks_to_price_exactly() {
    for &(ts, d, tu) in CONFIGS {
        for n in 0..=EXHAUSTIVE {
            let p = oracle_price(n, tu, d);
            assert_eq!(price_to_ticks(p, ts), n, "ts={ts} d={d} n={n} p={p}");
        }
        let mut rng = Lcg(0xC0FFEE ^ (d as u64) << 8 ^ tu as u64);
        for _ in 0..SAMPLES {
            let n = (rng.next() % 2_000_000_000) as i64;
            let p = oracle_price(n, tu, d);
            assert_eq!(price_to_ticks(p, ts), n, "ts={ts} d={d} n={n} p={p}");
        }
    }
}

#[test]
fn round_trip_is_stable_at_tick_boundaries_and_one_ulp_either_side() {
    // A price that is a hair off a tick boundary (float noise from upstream arithmetic) must still
    // land on the intended tick.
    for &(ts, d, tu) in CONFIGS {
        let mut rng = Lcg(0xBADC0DE ^ d as u64);
        for _ in 0..100_000 {
            let n = (rng.next() % 5_000_000) as i64;
            let p = oracle_price(n, tu, d);
            let up = f64::from_bits(p.to_bits() + 1);
            let down = f64::from_bits(p.to_bits() - if p > 0.0 { 1 } else { 0 });
            assert_eq!(price_to_ticks(up, ts), n, "ts={ts} n={n} +1ulp");
            assert_eq!(price_to_ticks(down, ts), n, "ts={ts} n={n} -1ulp");
        }
    }
}

#[test]
fn off_grid_prices_snap_to_the_nearest_tick_and_are_monotonic() {
    for &(ts, d, tu) in CONFIGS {
        let base = 100_000i64;
        // sweep across ten ticks in 1/100-tick steps: result must be non-decreasing and
        // within half a tick of the input.
        let mut prev = i64::MIN;
        for step in 0..1000 {
            let n_f = base as f64 + step as f64 / 100.0;
            let p = oracle_price(base, tu, d) + (step as f64 / 100.0) * ts;
            let t = price_to_ticks(p, ts);
            assert!(t >= prev, "non-monotonic at ts={ts} step={step}");
            prev = t;
            assert!((t as f64 - n_f).abs() <= 0.5 + 1e-6, "ts={ts} step={step}: {t} vs {n_f}");
        }
    }
}

#[test]
fn sl_and_tp_are_exact_tick_offsets_in_both_directions() {
    for &(ts, d, tu) in CONFIGS {
        let mut rng = Lcg(0xFEED ^ (d as u64) << 4 ^ tu as u64);
        for _ in 0..50_000 {
            let entry_ticks = (rng.next() % 3_000_000) as i64 + 1_000;
            let dist = (rng.next() % 5_000) as i64 + 1;
            let entry = oracle_price(entry_ticks, tu, d);
            for is_buy in [true, false] {
                let sl = calculate_sl_ticks(entry, dist, is_buy, ts, d);
                let tp = calculate_tp_ticks(entry, dist, is_buy, ts, d);
                let (sl_n, tp_n) = if is_buy {
                    (entry_ticks - dist, entry_ticks + dist)
                } else {
                    (entry_ticks + dist, entry_ticks - dist)
                };
                assert_eq!(sl.to_bits(), oracle_price(sl_n, tu, d).to_bits(), "SL ts={ts} buy={is_buy}");
                assert_eq!(tp.to_bits(), oracle_price(tp_n, tu, d).to_bits(), "TP ts={ts} buy={is_buy}");
                assert_eq!(price_to_ticks(sl, ts), sl_n);
                assert_eq!(price_to_ticks(tp, ts), tp_n);
                // distance sign is irrelevant
                assert_eq!(calculate_sl_ticks(entry, -dist, is_buy, ts, d).to_bits(), sl.to_bits());
            }
        }
    }
}

#[test]
fn well_known_prices_convert_exactly() {
    assert_eq!(price_to_ticks(1.08505, 0.00001), 108505);
    assert_eq!(ticks_to_price(108505, 0.00001, 5), 1.08505);
    assert_eq!(price_to_ticks(1.1, 0.1), 11);
    assert_eq!(price_to_ticks(0.3, 0.1), 3); // 0.3/0.1 = 2.9999999999999996 in raw f64
    assert_eq!(price_to_ticks(0.7, 0.1), 7); // 0.7/0.1 = 6.999999999999999
    assert_eq!(ticks_to_price(3, 0.1, 1), 0.3);
    assert_eq!(price_to_ticks(4500.25, 0.25), 18001);
    assert_eq!(price_to_ticks(64321.55, 0.01), 6_432_155);
    assert_eq!(ticks_to_price(6_432_155, 0.01, 2), 64321.55);
    assert_eq!(calculate_sl_ticks(1.08505, 100, true, 0.00001, 5), 1.08405);
    assert_eq!(calculate_tp_ticks(1.08505, 100, true, 0.00001, 5), 1.08605);
    assert_eq!(calculate_sl_ticks(1.08505, 100, false, 0.00001, 5), 1.08605);
}

#[test]
fn degenerate_inputs_are_handled_without_panicking() {
    assert_eq!(price_to_ticks(1.0, 0.0), 0);
    assert_eq!(price_to_ticks(1.0, -0.1), 0);
    assert_eq!(price_to_ticks(f64::NAN, 0.1), 0);
    assert_eq!(price_to_ticks(f64::INFINITY, 0.1), 0);
    assert_eq!(price_to_ticks(f64::NEG_INFINITY, 0.1), 0);
    assert_eq!(price_to_ticks(1e300, 1e-5), i64::MAX, "saturates instead of wrapping");
    assert_eq!(price_to_ticks(-1e300, 1e-5), i64::MIN);
    assert_eq!(price_to_ticks(0.0, 0.01), 0);
    assert_eq!(ticks_to_price(0, 0.01, 2), 0.0);
    // must not panic on extreme tick counts / digits
    let _ = ticks_to_price(i64::MAX, 0.01, 2);
    let _ = ticks_to_price(i64::MIN, 0.01, 2);
    let _ = ticks_to_price(1, 0.01, 40);
    // Extreme distances must keep protection on the correct SIDE of entry (regression: in release
    // builds `i64::MIN.abs()` wrapped negative, flipping a buy's stop above entry).
    for dist in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert!(calculate_sl_ticks(1.0, dist, true, 0.01, 2) <= 1.0, "buy SL below entry, dist={dist}");
        assert!(calculate_sl_ticks(1.0, dist, false, 0.01, 2) >= 1.0, "sell SL above entry, dist={dist}");
        assert!(calculate_tp_ticks(1.0, dist, true, 0.01, 2) >= 1.0, "buy TP above entry, dist={dist}");
        assert!(calculate_tp_ticks(1.0, dist, false, 0.01, 2) <= 1.0, "sell TP below entry, dist={dist}");
    }
    // Saturated entry (absurd price) must not overflow either.
    let _ = calculate_sl_ticks(1e300, 5, true, 1e-5, 5);
    let _ = calculate_tp_ticks(1e300, 5, true, 1e-5, 5);
}
