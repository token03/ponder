//! Port of tests/test_pob.py.

mod common;

use ponder_game::pob;

fn build_a() -> String {
    std::fs::read_to_string(common::fixtures().join("build_a.xml")).unwrap()
}

#[test]
fn code_roundtrip() {
    let xml = build_a();
    let code = pob::encode(&xml);
    assert!(!code.contains('+') && !code.contains('/'));
    assert_eq!(pob::decode(&code).unwrap(), xml);
}

#[test]
fn decode_ignores_whitespace_and_padding() {
    let xml = build_a();
    let code = pob::encode(&xml);
    let spaced: String = code.chars().flat_map(|c| if c == 'A' { vec![' ', c, '\n'] } else { vec![c] }).collect();
    assert_eq!(pob::decode(&spaced).unwrap(), xml);
    assert_eq!(pob::decode(code.trim_end_matches('=')).unwrap(), xml);
}

#[test]
fn load_from_code_or_file() {
    let xml = build_a();
    let dir = std::env::temp_dir().join(format!("ponder-pob-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let code_file = dir.join("build.txt");
    std::fs::write(&code_file, pob::encode(&xml)).unwrap();
    let no_fetch = |_: &str| -> Result<String, String> { panic!("no download expected") };
    for source in [common::fixtures().join("build_a.xml").to_string_lossy().to_string(), code_file.to_string_lossy().to_string(), pob::encode(&xml)] {
        let build = pob::load_with(&source, no_fetch).unwrap();
        assert_eq!(build.ascendancy, "Elementalist");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn share_urls_map_to_raw_urls() {
    assert_eq!(pob::raw_url("https://pobb.in/Jj2d38SXQtTC").as_deref(), Some("https://pobb.in/Jj2d38SXQtTC/raw"));
    assert_eq!(pob::raw_url(" https://pastebin.com/abc123/ ").as_deref(), Some("https://pastebin.com/raw/abc123"));
    assert_eq!(pob::raw_url("https://pastebin.com/raw/abc123").as_deref(), Some("https://pastebin.com/raw/abc123"));
    assert_eq!(pob::raw_url("https://example.com/x"), None);
    let seen = std::cell::RefCell::new(Vec::new());
    let build = pob::load_with("https://pobb.in/abc", |url| {
        seen.borrow_mut().push(url.to_string());
        Ok(format!("  {}\n", pob::encode(&build_a())))
    })
    .unwrap();
    assert_eq!(seen.into_inner(), ["https://pobb.in/abc/raw"]);
    assert_eq!(build.class_name, "Witch");
}

#[test]
fn parse_build() {
    let build = pob::parse(&build_a()).unwrap();
    assert_eq!((build.class_name.as_str(), build.level, build.main_skill.as_deref()), ("Witch", 94, Some("Fireball")));
    let slots: std::collections::BTreeSet<_> = build.items.iter().filter_map(|i| i.slot.as_deref()).collect();
    assert_eq!(slots, ["Amulet", "Boots", "Helmet"].into_iter().collect());
    let first: Vec<_> = build.items[0].text.lines().take(3).collect();
    assert_eq!(first, ["Rarity: RARE", "Doom Crown", "Hubris Circlet"]);
}

#[test]
fn load_text_takes_xml_or_code() {
    let xml = build_a();
    assert_eq!(pob::load_text(&xml).unwrap(), pob::load_text(&pob::encode(&xml)).unwrap());
    assert!(pob::load_text("definitely not a code!").is_err());
}
