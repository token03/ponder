//! demand: archetype helpers on small made-up inputs.

use ponder::demand::{Archetype, Counter};

#[test]
fn niche_and_usual_tier() {
    let mut a = Archetype::new("g", vec!["x".into()], 25, 40);
    a.lift = 1.0;
    assert_eq!(a.niche(), 0.0);
    a.lift = std::f64::consts::E;
    assert!((a.niche() - 5.0).abs() < 1e-12);
    for t in [1, 1, 2, 3] {
        a.tiers.entry("x".into()).or_default().add(t);
    }
    assert_eq!(a.usual_tier("x", 0.5), Some(1));
    assert_eq!(a.usual_tier("x", 0.75), Some(2));
    assert_eq!(a.usual_tier("x", 1.0), Some(3));
    assert_eq!(a.usual_tier("y", 0.5), None);
}

#[test]
fn counter_most_common_keeps_insertion_order_on_ties() {
    let mut c = Counter::new();
    for k in ["b", "a", "c", "a", "c"] {
        c.add(k.to_string());
    }
    c.add("d".to_string());
    assert_eq!(
        c.most_common()
            .iter()
            .map(|(k, n)| (k.as_str(), *n))
            .collect::<Vec<_>>(),
        [("a", 2), ("c", 2), ("b", 1), ("d", 1)]
    );
}
