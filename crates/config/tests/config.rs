use plan_viewer_config::{Color, Config, Mode};

#[test]
fn vc_9_layers_merge_tables_and_replace_arrays() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("user.toml");
    let worktree = root.path().join(".plan-viewer.toml");
    let explicit = root.path().join("explicit.toml");
    std::fs::write(&user, "[theme.roles]\naccent='red'\n[[status]]\nname='custom'\ncolor='red'\n").unwrap();
    std::fs::write(
        &worktree,
        "chip=[]\n[theme.roles]\naccent='green'\nheadings=['blue']\n[[status]]\nname='shipped'\naliases=['released']\ncolor='magenta'\n",
    )
    .unwrap();
    std::fs::write(&explicit, "[theme.roles]\naccent='cyan'\n").unwrap();
    let config = Config::load_layers(Some(&user), root.path(), Some(&explicit)).unwrap();
    let theme = config.resolve(Mode::Dark).unwrap();
    assert_eq!(theme.roles.accent, Color::Ansi(plan_viewer_config::Ansi::Cyan));
    assert_eq!(theme.roles.headings.len(), 1);
    assert_eq!(config.parse.statuses.len(), 1);
    assert_eq!(config.parse.statuses[0].name, "shipped");
    assert!(config.parse.chips.is_empty());
}

#[test]
fn vc_9_validation_errors_name_the_defining_file_and_key() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".plan-viewer.toml");
    for (source, key) in [
        ("[theme]\nmode='rainbow'", "theme.mode"),
        ("[theme]\nname='missing'", "theme.name"),
        ("[theme]\nlight_name='missing'", "theme.light_name"),
        ("[theme]\nstrange=true", "theme.strange"),
        ("strange=true", "strange"),
        ("[theme.roles]\nstrange='blue'", "theme.roles.strange"),
        ("[theme.roles.xref]\ngg='blue'", "theme.roles.xref.gg"),
        ("[theme.roles]\naccent='missing'", "theme.roles.accent"),
        ("[theme.roles.xref]\nkd='missing'", "theme.roles.xref.kd"),
        ("[theme.roles]\nheadings=['missing']", "theme.roles.headings"),
        ("[theme.roles]\nheadings=[]", "theme.roles.headings"),
        ("[theme.palette]\nblue='#12345g'", "theme.palette.blue"),
        ("[theme.palette.dark]\nblue='rgb(256,0,0)'", "theme.palette.dark.blue"),
        ("[[chip]]\nlabel='R'\nkind='unknown'\nsections=['risks']", "chip[0].kind"),
        (
            "[[chip]]\nlabel='R'\nkind='headings'\nsections=['risks']\ncolor='blue'",
            "chip[0].color",
        ),
        ("[[status]]\nname='x'\ncolor='missing'", "status[0].color"),
        ("[[status]]\nname='x'\ncolor='blue'\naliases=['X']", "status[0].aliases"),
        (
            "[[status]]\nname='x'\ncolor='blue'\naliases=['released']\n[[status]]\nname='RELEASED'\ncolor='green'",
            "status[1].name",
        ),
        (
            "[[status]]\nname='x'\ncolor='blue'\naliases=['released']\n[[status]]\nname='y'\ncolor='green'\naliases=['Released']",
            "status[1].aliases",
        ),
        ("[theme]\ncode_dark='missing.tmTheme'", "theme.code_dark"),
        ("[theme]\ncode_light='missing'", "theme.code_light"),
    ] {
        std::fs::write(&file, source).unwrap();
        let error = Config::load_layers(None, root.path(), None).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
        assert_eq!(error.to_string().lines().count(), 1);
    }
    std::fs::remove_file(&file).unwrap();
    Config::load_layers(Some(&root.path().join("absent-user.toml")), root.path(), None).unwrap();
    let missing = root.path().join("absent-explicit.toml");
    let error = Config::load_layers(None, root.path(), Some(&missing)).unwrap_err();
    assert_eq!(error.file, missing);
    assert_eq!(error.key, "--config");
}

#[test]
fn vc_9_colour_syntax_and_palette_validation() {
    use plan_viewer_config::Ansi;
    for (source, expected) in [
        ("#123aBC", Color::Rgb(0x12, 0x3a, 0xbc)),
        ("rgb(1, 2, 255)", Color::Rgb(1, 2, 255)),
        ("reset", Color::Reset),
        ("white", Color::Ansi(Ansi::White)),
        ("bright_white", Color::Ansi(Ansi::BrightWhite)),
    ] {
        assert_eq!(Color::parse(source), Some(expected));
    }
    for source in [
        "#123",
        "#gg0000",
        "#ééé",
        "rgb(256,0,0)",
        "rgb(-1,0,0)",
        "rgb(1,2)",
        "rgb(1,2,3,4)",
        "rgb(1,2,3)tail",
        "blueish",
    ] {
        assert_eq!(Color::parse(source), None, "{source}");
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(".plan-viewer.toml"),
        "[theme.palette]\nbrand='reset'\n[theme.roles]\naccent='brand'",
    )
    .unwrap();
    assert_eq!(
        Config::load_layers(None, root.path(), None)
            .unwrap()
            .resolve(Mode::Dark)
            .unwrap()
            .roles
            .accent,
        Color::Reset
    );
}

#[test]
fn vc_9_empty_unknown_table_errors_name_the_file() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".plan-viewer.toml");
    std::fs::write(&file, "[theme.unknown]\n").unwrap();
    let error = Config::load_layers(None, root.path(), None).unwrap_err();
    assert_eq!(error.file, file);
    assert!(error.key.contains("unknown"));
}

#[test]
fn vc_9_palette_shape_errors_identify_the_exact_key_and_layer() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".plan-viewer.toml");
    for (source, key) in [
        ("[theme.palette]\nblue=123", "theme.palette.blue"),
        ("[theme.palette.autumn]\nblue='#123456'", "theme.palette.autumn"),
        ("[theme.palette.light]\nblue=false", "theme.palette.light.blue"),
        ("[theme.palette]\nlight='blue'", "theme.palette.light"),
    ] {
        std::fs::write(&file, source).unwrap();
        let error = Config::load_layers(None, root.path(), None).unwrap_err();
        assert_eq!(error.file, file);
        assert_eq!(error.key, key, "{source}: {error}");
    }
}
