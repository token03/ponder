//! mods tests, and the match goldens (fixtures, texts, ladder).

mod common;

use common::{db, expected, item_json, match_json, normalized, same_order};
use ponder_game::item::parse_item;
use ponder_game::mods::{IdMods, fits, line_values, normalize, template_ranges};
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn normalize_and_ranges() {
    assert_eq!(
        normalize("+(100-114) to maximum Life"),
        "+# to maximum life"
    );
    assert_eq!(normalize("+105 to maximum Life"), "+# to maximum life");
    assert_eq!(
        template_ranges("Adds (5-7) to (11-13) Fire Damage"),
        [(5.0, 7.0), (11.0, 13.0)]
    );
    assert_eq!(template_ranges("(-10--5)% to X"), [(-10.0, -5.0)]);
}

#[test]
fn values_and_fits() {
    let mut line = ponder_game::item::ModLine::new("+(42-45)% to Lightning Resistance");
    assert_eq!(line_values(&line), [43.5]);
    line.markers.insert("range".into(), "0".into());
    assert_eq!(line_values(&line), [42.0]);
    assert!(fits(&[41.6], &[(42.0, 45.0)]) && !fits(&[41.4], &[(42.0, 45.0)]));
    assert!(!fits(&[1.0, 2.0], &[(0.0, 5.0)]));
}

fn item(text: &str) -> ponder_game::item::Item {
    parse_item(text).unwrap()
}

#[test]
fn tiers_and_sources() {
    common::setup();
    let m = db().match_item(&item(
        "Rarity: RARE\nX\nHubris Circlet\nShaper Item\nItem Level: 86\nImplicits: 0\n\
         +135 to maximum Life\n+44% to Lightning Resistance\n{crafted}+23% to Lightning Resistance\n",
    ));
    let by_text = |t: &str| m.mods.iter().find(|x| x.lines[0].text == t).unwrap();
    let life = by_text("+135 to maximum Life");
    assert_eq!(
        (life.kind.as_str(), life.source, life.tier),
        ("prefix", "natural", Some(1))
    );
    assert_eq!(by_text("+44% to Lightning Resistance").tier, Some(2));
    assert_eq!(by_text("+23% to Lightning Resistance").source, "crafted");
    assert!(m.unmatched.is_empty());
}

#[test]
fn hybrid_mod_is_matched_as_one() {
    common::setup();
    let m = db().match_item(&item(
        "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n\
         40% increased Energy Shield\n16% increased Stun and Block Recovery\n",
    ));
    assert!(m.mods.len() == 1 && m.mods[0].lines.len() == 2);
}

#[test]
fn influence_mod_needs_influence() {
    common::setup();
    let text = |inf: &str| {
        format!(
            "Rarity: RARE\nX\nHubris Circlet\n{inf}Item Level: 86\nImplicits: 0\nAdds 20 to 38 Cold Damage to Spells\n"
        )
    };
    let shaper = &db().match_item(&item(&text("Shaper Item\n"))).mods[0];
    let plain = &db().match_item(&item(&text(""))).mods[0];
    assert_eq!(shaper.source, "natural");
    assert_eq!(plain.source, "special"); // not natural without the influence
}

#[test]
fn unknown_lines_are_reported() {
    common::setup();
    let m = db().match_item(&item(
        "Rarity: RARE\nX\nHubris Circlet\nImplicits: 0\nUse a fossil here\n+999 to maximum Life\n",
    ));
    assert_eq!(
        m.notes.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["Use a fossil here"]
    );
    assert_eq!(
        m.unmatched
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["+999 to maximum Life"]
    );
}

#[test]
fn summed_line_split_into_hybrid_and_single() {
    common::setup();
    let m = db().match_item(&item(
        "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n\
         136% increased Energy Shield\n16% increased Stun and Block Recovery\n",
    ));
    assert!(m.unmatched.is_empty() && m.mods.len() == 2);
    assert!(m.mods.iter().all(|x| x.merged));
}

#[test]
fn unknown_base_leaves_everything_unmatched() {
    common::setup();
    let m = db().match_item(&item(
        "Rarity: RARE\nX\nNo Such Base\nImplicits: 0\n+10 to Strength\n",
    ));
    assert!(m.base_id.is_none() && m.mods.is_empty() && m.unmatched.len() == 1);
}

/// Compare Rust matches to goldens; returns (identical, total) and prints every difference.
fn compare(
    label: &str,
    pairs: impl Iterator<Item = (String, Value, Value)>,
    strict_tags: impl Fn(&Value) -> bool,
) -> (usize, usize, usize) {
    let (mut same, mut total, mut ordered) = (0, 0, 0);
    for (what, got, want) in pairs {
        total += 1;
        let tags = strict_tags(&want);
        if normalized(got.clone(), tags) == normalized(want.clone(), tags) {
            same += 1;
            ordered += same_order(&got, &want) as usize;
        } else {
            eprintln!("[{label}] differs: {what}");
            let (g, w) = (normalized(got, tags), normalized(want, tags));
            for key in g.as_object().unwrap().keys() {
                if g[key] == w[key] {
                    continue;
                }
                if let (Some(a), Some(b)) = (g[key].as_array(), w[key].as_array()) {
                    for x in a.iter().filter(|x| !b.contains(x)) {
                        eprintln!("    {key} only in actual:   {x}");
                    }
                    for x in b.iter().filter(|x| !a.contains(x)) {
                        eprintln!("    {key} only in expected: {x}");
                    }
                } else {
                    eprintln!("    {key}: actual {} expected {}", g[key], w[key]);
                }
            }
        }
    }
    eprintln!("[{label}] identical {same}/{total} (same mod order {ordered})");
    (same, total, ordered)
}

#[test]
fn texts_match_fixtures() {
    common::setup();
    let texts = expected("texts");
    let pairs = texts
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e.get("match").is_some())
        .map(|e| {
            let item = parse_item(e["text"].as_str().unwrap()).unwrap();
            (
                e["text"].to_string(),
                match_json(&db().match_item(&item)),
                e["match"].clone(),
            )
        });
    let (same, total, _) = compare("texts", pairs, |_| true);
    assert_eq!(same, total);
}

#[test]
fn fixture_builds_match_fixtures() {
    common::setup();
    let builds = expected("fixtures");
    let mut pairs = Vec::new();
    for b in builds.as_array().unwrap() {
        let path = common::fixtures().join(b["file"].as_str().unwrap());
        let build =
            ponder_game::pob::load_with(path.to_str().unwrap(), |_| Err("no network".into()))
                .unwrap();
        assert_eq!(build.class_name, b["class_name"].as_str().unwrap());
        assert_eq!(build.ascendancy, b["ascendancy"].as_str().unwrap());
        assert_eq!(Some(build.level), b["level"].as_i64());
        assert_eq!(build.main_skill.as_deref(), b["main_skill"].as_str());
        assert_eq!(build.items.len(), b["items"].as_array().unwrap().len());
        for (pi, g) in build.items.iter().zip(b["items"].as_array().unwrap()) {
            assert_eq!(pi.id, g["id"].as_str().unwrap());
            assert_eq!(pi.slot.as_deref(), g["slot"].as_str());
            let sets: Vec<&str> = pi.sets.iter().map(String::as_str).collect();
            let want: Vec<&str> = g["sets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            assert_eq!(sets, want);
            let Some(want_item) = g.get("item") else {
                assert!(parse_item(&pi.text).is_err());
                continue;
            };
            let item = parse_item(&pi.text).unwrap();
            assert_eq!(item_json(&item), *want_item, "{} item {}", b["file"], pi.id);
            pairs.push((
                format!("{} item {}", b["file"], pi.id),
                match_json(&db().match_item(&item)),
                g["match"].clone(),
            ));
        }
    }
    let (same, total, _) = compare("fixtures", pairs.into_iter(), |_| true);
    assert_eq!(same, total);
}

#[test]
fn ladder_match_ids_match_fixtures() {
    common::setup();
    let ladder = expected("ladder");
    let pairs = ladder["items"].as_array().unwrap().iter().map(|e| {
        let ids = |k: &str| -> Vec<String> {
            e["mods"][k]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect()
        };
        let mods = IdMods {
            explicit: ids("explicit"),
            fractured: ids("fractured"),
            crafted: ids("crafted"),
        };
        let influences: BTreeSet<String> = e["influences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        let m = db().match_ids(
            e["base"].as_str().unwrap(),
            &mods,
            e["ilvl"].as_i64(),
            &influences,
            "RARE",
            "",
            &Default::default(),
        );
        (e["base"].to_string(), match_json(&m), e["match"].clone())
    });
    let (same, total, ordered) = compare("ladder", pairs, |_| true);
    assert_eq!(same, total);
    assert_eq!(ordered, total, "mod order must match too");
}

#[test]
fn match_ids_skips_unknown_ids() {
    common::setup();
    let mods = IdMods {
        explicit: vec!["IncreasedLife7".into(), "NoSuchMod".into(), "".into()],
        ..Default::default()
    };
    let m = db().match_ids(
        "Hubris Circlet",
        &mods,
        Some(86),
        &BTreeSet::new(),
        "RARE",
        "",
        &Default::default(),
    );
    assert_eq!(m.mods.len(), 1);
    assert_eq!(m.mods[0].mod_id, "IncreasedLife7");
    let none = db().match_ids(
        "Nope",
        &mods,
        None,
        &BTreeSet::new(),
        "RARE",
        "",
        &Default::default(),
    );
    assert!(none.base_id.is_none() && none.mods.is_empty());
}

/// Texts built from real mods (hybrids, summed lines, crafted lines): parse and match both agree with Python.
#[test]
fn synthetic_items_match_fixtures() {
    common::setup();
    let synth = expected("synth");
    let mut pairs = Vec::new();
    for e in synth.as_array().unwrap() {
        let item = parse_item(e["text"].as_str().unwrap()).unwrap();
        assert_eq!(item_json(&item), e["item"]);
        pairs.push((
            e["text"].to_string(),
            match_json(&db().match_item(&item)),
            e["match"].clone(),
        ));
    }
    let (same, total, ordered) = compare("synth", pairs.into_iter(), |_| true);
    assert_eq!(same, total);
    assert_eq!(ordered, total, "mod order must match too");
}
