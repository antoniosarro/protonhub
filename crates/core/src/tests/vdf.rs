use super::*;

const CONFIG: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_hotfix\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"75\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t\t\"path\"\t\t\"C:\\\\Games \\\"x\\\"\"\n\t\t\t}\n\t\t}\n\t}\n}\n";

#[test]
fn round_trips_steams_layout() {
    let obj = parse(CONFIG).unwrap();
    assert_eq!(to_string(&obj), CONFIG);
    let steam = obj
        .path(&["installconfigstore", "software", "valve", "steam"])
        .unwrap();
    assert_eq!(steam.str("path"), Some(r#"C:\Games "x""#));
    let default = steam.path(&["CompatToolMapping", "0"]).unwrap();
    assert_eq!(default.str("name"), Some("proton_hotfix"));
}

#[test]
fn edits_in_place() {
    let mut obj = parse(CONFIG).unwrap();
    let map = obj.path_mut(&[
        "InstallConfigStore",
        "Software",
        "Valve",
        "Steam",
        "CompatToolMapping",
    ]);
    let mut entry = Obj::default();
    entry.set("name", Value::Str("GE-Proton11-7".into()));
    map.set("409710", Value::Obj(entry));
    assert!(map.remove("0"));
    assert!(!map.remove("0"));
    let text = to_string(&obj);
    assert!(text.contains(
        "\t\t\t\t\t\"409710\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton11-7\"\n"
    ));
    assert!(!text.contains("proton_hotfix"));
    // Everything else is untouched.
    assert!(
        text.ends_with("\t\t\t\t\"path\"\t\t\"C:\\\\Games \\\"x\\\"\"\n\t\t\t}\n\t\t}\n\t}\n}\n")
    );
}

#[test]
fn reads_compatibilitytool_vdf() {
    let text = r#"
        "compatibilitytools"
        {
          "compat_tools"
          {
            "GE-Proton11-7" // Internal name of this tool
            {
              "install_path" "."
              "display_name" "GE-Proton11-7"
              "from_oslist"  "windows"
            }
          }
        }
        "#;
    let obj = parse(text).unwrap();
    let tools = obj.path(&["compatibilitytools", "compat_tools"]).unwrap();
    assert_eq!(tools.0[0].0, "GE-Proton11-7");
    assert_eq!(
        tools.obj("GE-Proton11-7").unwrap().str("install_path"),
        Some(".")
    );
}

#[test]
fn reports_where_it_breaks() {
    assert_eq!(parse("\"a\"\n{\n\"b\"").unwrap_err().line, 3);
    assert!(parse("\"a\" {").is_err());
    assert!(parse("}").is_err());
    assert!(parse("\"a\" \"b").is_err());
}

#[test]
fn reads_binary_shortcuts() {
    let mut b = vec![0x00];
    b.extend(b"shortcuts\0");
    b.push(0x00);
    b.extend(b"0\0");
    b.push(0x02);
    b.extend(b"appid\0");
    b.extend((-1_234_567_890i32).to_le_bytes());
    b.push(0x01);
    b.extend(b"AppName\0My Game\0");
    b.extend([0x08, 0x08, 0x08]);
    let obj = parse_binary(&b).unwrap();
    let game = obj.path(&["shortcuts", "0"]).unwrap();
    assert_eq!(game.get("AppID"), Some(&Value::Int(-1_234_567_890)));
    assert_eq!(game.str("appname"), Some("My Game"));
    assert!(parse_binary(&b[..b.len() - 6]).is_err());
    assert_eq!(to_binary(&obj), b, "it writes back the same");
}

#[test]
fn reads_binary_keys_from_a_table() {
    let keys: Vec<String> = ["appinfo", "common", "name", "gameid"]
        .map(String::from)
        .to_vec();
    let mut b = vec![0x00];
    b.extend(0u32.to_le_bytes());
    b.push(0x00);
    b.extend(1u32.to_le_bytes());
    b.push(0x01);
    b.extend(2u32.to_le_bytes());
    b.extend(b"Hades\0");
    b.push(0x07);
    b.extend(3u32.to_le_bytes());
    b.extend(1_145_360u64.to_le_bytes());
    b.extend([0x08, 0x08, 0x08]);
    let obj = parse_binary_keyed(&b, &keys).unwrap();
    let common = obj.path(&["appinfo", "common"]).unwrap();
    assert_eq!(common.str("name"), Some("Hades"));
    assert_eq!(common.get("gameid"), Some(&Value::Int(1_145_360)));
    assert!(
        parse_binary_keyed(&b, &keys[..2]).is_err(),
        "a key past the table"
    );
}
