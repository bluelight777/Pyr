//! The little statistics the starting odds need.

/// The standard normal's cumulative distribution (Abramowitz and Stegun 7.1.26: good to 1e-7).
pub fn phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    if x >= 0.0 { 0.5 * (1.0 + erf) } else { 0.5 * (1.0 - erf) }
}

/// The chance a normal of mean `mean` and deviation `sd` lands in `lo..hi` (None: no bound).
pub fn normal_between(mean: f64, sd: f64, lo: Option<f64>, hi: Option<f64>) -> f64 {
    let sd = sd.max(1e-9);
    let up = hi.map_or(1.0, |h| phi((h - mean) / sd));
    let down = lo.map_or(0.0, |l| phi((l - mean) / sd));
    (up - down).max(0.0)
}

/// The chance of exactly `k` of something that happens `lambda` times on average.
pub fn poisson(k: u32, lambda: f64) -> f64 {
    let mut p = (-lambda).exp();
    for i in 1..=k {
        p *= lambda / i as f64;
    }
    p
}

/// The chance of `lo..=hi` of them (None: no upper bound).
pub fn poisson_between(lambda: f64, lo: u32, hi: Option<u32>) -> f64 {
    match hi {
        Some(hi) => (lo..=hi).map(|k| poisson(k, lambda)).sum(),
        None => (1.0 - (0..lo).map(|k| poisson(k, lambda)).sum::<f64>()).max(0.0),
    }
}

/// The chance a bookmaker's American odds stand for (-230: 0.697; +190: 0.345), its margin in.
pub fn american(odds: i32) -> f64 {
    if odds < 0 { -odds as f64 / (-odds as f64 + 100.0) } else { 100.0 / (odds as f64 + 100.0) }
}

/// Two sides' chances with the bookmaker's margin taken out (they add up to 1).
pub fn fair(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    if sum <= 0.0 { (0.5, 0.5) } else { (a / sum, b / sum) }
}

/// Starting odds as the feed carries them: none under 3% or over 97%, adding up to 1, to
/// three places.
pub fn prior(p: &[f64]) -> Vec<f64> {
    let n = p.len().max(1) as f64;
    let sum: f64 = p.iter().map(|x| x.max(0.0)).sum();
    let mut out: Vec<f64> = if sum <= 0.0 { vec![1.0 / n; p.len()] } else { p.iter().map(|x| (x.max(0.0) / sum).clamp(0.03, 0.97)).collect() };
    let sum: f64 = out.iter().sum();
    for x in out.iter_mut() {
        *x = (*x / sum * 1000.0).round() / 1000.0;
    }
    // (What rounding left over goes on the likeliest.)
    let left = 1.0 - out.iter().sum::<f64>();
    if let Some(top) = out.iter_mut().max_by(|a, b| a.total_cmp(b)) {
        *top = ((*top + left) * 1000.0).round() / 1000.0;
    }
    out
}

/// A number with thousands separated: 1234567 as "1,234,567".
pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_normal_and_poisson_sums_are_right() {
        assert!((phi(0.0) - 0.5).abs() < 1e-7);
        assert!((phi(1.0) - 0.841_344_7).abs() < 1e-6);
        assert!((phi(-1.96) - 0.025).abs() < 1e-4);
        assert!((normal_between(50.0, 2.8, Some(48.5), Some(51.5)) - 0.408).abs() < 0.005);
        assert!((normal_between(0.0, 1.0, None, None) - 1.0).abs() < 1e-9);
        assert!((poisson(0, 0.75) - 0.472_366_6).abs() < 1e-6);
        let all: f64 = poisson_between(2.6, 0, Some(1)) + poisson_between(2.6, 2, Some(2)) + poisson_between(2.6, 3, Some(3)) + poisson_between(2.6, 4, None);
        assert!((all - 1.0).abs() < 1e-9);
    }

    #[test]
    fn odds_become_chances() {
        assert!((american(-230) - 0.696_97).abs() < 1e-4);
        assert!((american(190) - 0.344_83).abs() < 1e-4);
        let (home, away) = fair(american(-230), american(190));
        assert!((home - 0.669).abs() < 0.001 && (home + away - 1.0).abs() < 1e-12);
        assert_eq!(prior(&[0.669, 0.331]), vec![0.669, 0.331]);
        assert_eq!(prior(&[0.999, 0.001]), vec![0.97, 0.03]);
        let p = prior(&[0.105, 0.19, 0.41, 0.19, 0.105]);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{p:?}");
        assert_eq!(prior(&[0.0, 0.0]), vec![0.5, 0.5]);
        assert_eq!(commas(1_234_567), "1,234,567");
        assert_eq!(commas(999), "999");
    }
}
