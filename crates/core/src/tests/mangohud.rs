use super::*;

#[test]
fn settings_load_and_are_whole() {
    let groups = groups();
    let all: Vec<_> = groups.iter().flat_map(|g| &g.setting).collect();
    assert!(all.len() > 20);
    for s in &all {
        assert_eq!(
            all.iter().filter(|t| t.key == s.key).count(),
            1,
            "{} twice",
            s.key
        );
        match s.kind {
            Kind::Switch => assert!(["0", "1"].contains(&s.default.as_str()), "{}", s.key),
            Kind::Number => {
                let d: f64 = s.default.parse().unwrap();
                assert!(s.min < s.max && s.step > 0.0, "{}", s.key);
                assert!((s.min..=s.max).contains(&d), "{}", s.key);
            }
            Kind::Choice => assert!(s.choices.contains(&s.default), "{}", s.key),
            Kind::Color => assert!(
                s.default.len() == 6 && u32::from_str_radix(&s.default, 16).is_ok(),
                "{}",
                s.key
            ),
            Kind::Text => {}
        }
    }
}

#[test]
fn keeps_unknown_lines_and_leaves_defaults_out() {
    let text =
        "# mine\nfps_sampling_period=300\ncpu_temp\nfont_size = 30\n\nfps_limit=60\nfont_size=32\n";
    let mut c = Config::parse(text);
    assert_eq!(c.to_string().replace(" = ", "="), text.replace(" = ", "="));
    assert_eq!(c.get("cpu_temp"), Some("1"));
    assert_eq!(c.get("font_size"), Some("32"), "the last one counts");
    assert_eq!(c.get("gpu_temp"), None);

    let all: Vec<Setting> = groups().into_iter().flat_map(|g| g.setting).collect();
    let setting = |key: &str| all.iter().find(|s| s.key == key).unwrap();
    setting("font_size").set(&mut c, "24");
    setting("cpu_temp").set(&mut c, "0");
    setting("gpu_stats").set(&mut c, "0");
    setting("position").set(&mut c, "bottom-right");
    setting("text_color").set(&mut c, "ffffff");
    setting("fps_limit").set(&mut c, "");
    assert_eq!(
        c.to_string(),
        "# mine\nfps_sampling_period=300\n\ngpu_stats=0\nposition=bottom-right\n"
    );
    assert_eq!(setting("font_size").value(&c), "24");
    assert_eq!(setting("gpu_stats").value(&c), "0");
    setting("gpu_stats").set(&mut c, "1");
    assert_eq!(c.get("gpu_stats"), None);
    assert!(!is_on("false") && !is_on("0") && is_on("1") && is_on(" 2 "));
    c.set("fps", Some("1"));
    assert!(c.to_string().ends_with("\nfps\n"), "on is a bare key");
}

#[test]
fn the_newer_layout_shows_only_whats_listed() {
    let all: Vec<Setting> = groups().into_iter().flat_map(|g| g.setting).collect();
    let setting = |key: &str| all.iter().find(|s| s.key == key).unwrap();
    let mut c = Config::parse("legacy_layout=false\nram\n");
    assert_eq!(setting("fps").value(&c), "0");
    assert_eq!(setting("ram").value(&c), "1");
    assert_eq!(setting("text_outline").value(&c), "1", "not an element");
    setting("fps").set(&mut c, "1");
    setting("ram").set(&mut c, "0");
    assert_eq!(c.to_string(), "legacy_layout=false\nfps\n");
}
