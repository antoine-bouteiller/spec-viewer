use spec_viewer_config::{Color, Config, Mode};

#[test]
fn vc_7_layers_merge_tables_and_replace_arrays() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("user.toml");
    let worktree = root.path().join(".spec-viewer.toml");
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
    assert_eq!(theme.roles.accent, Color::Ansi(spec_viewer_config::Ansi::Cyan));
    assert_eq!(theme.roles.headings.len(), 1);
    assert_eq!(config.parse.statuses.len(), 1);
    assert_eq!(config.parse.statuses[0].name, "shipped");
    assert!(config.parse.chips.is_empty());
}

#[test]
fn vc_7_validation_errors_name_the_defining_file_and_key() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    for (source, key) in [
        ("[theme]\nmode='rainbow'", "theme.mode"),
        ("[theme\naccent='red'", "line 1"),
        ("[theme]\nmode='dark'\nname = [", "line 3"),
        ("[theme]\nmode='dark'\nname = 'terminal'\n[theme.roles\naccent='red'", "line 4"),
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
        ("[theme]\nname='catppuccin-mocha'", "theme.name"),
        ("[theme]\ncode_dark='custom.tmTheme'", "theme.code_dark"),
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
fn colour_syntax_and_palette_validation() {
    use spec_viewer_config::Ansi;
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
        root.path().join(".spec-viewer.toml"),
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
fn vc_7_empty_unknown_table_errors_name_the_file() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    std::fs::write(&file, "[theme.unknown]\n").unwrap();
    let error = Config::load_layers(None, root.path(), None).unwrap_err();
    assert_eq!(error.file, file);
    assert!(error.key.contains("unknown"));
}

#[test]
fn vc_7_palette_shape_errors_identify_the_exact_key_and_layer() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
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

#[test]
fn web_is_user_only_with_paths_relative_to_the_user_file() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("user/config.toml");
    std::fs::create_dir(root.path().join("user")).unwrap();
    std::fs::write(&user, "[web]\nbind='0.0.0.0'\ncert='cert.pem'\nkey='/abs/key.pem'\n").unwrap();
    let config = Config::load_layers(Some(&user), root.path(), None).unwrap();
    assert_eq!(config.web.bind, "0.0.0.0");
    assert_eq!(config.web.cert, Some(root.path().join("user/cert.pem")));
    assert_eq!(config.web.key, Some("/abs/key.pem".into()));
    assert_eq!(Config::default().web.bind, "127.0.0.1");

    let worktree = root.path().join(".spec-viewer.toml");
    let explicit = root.path().join("explicit.toml");
    std::fs::write(&explicit, "").unwrap();
    for file in [&worktree, &explicit] {
        std::fs::write(file, "[web]\nbind='0.0.0.0'\n").unwrap();
        let error = Config::load_layers(None, root.path(), Some(&explicit)).unwrap_err();
        assert_eq!((&error.file, error.key.as_str()), (file, "web"), "{error}");
        std::fs::write(file, "").unwrap();
    }

    for (source, key) in [
        ("[web]\nbind=''", "web.bind"),
        ("[web]\ncert='c.pem'", "web.cert"),
        ("[web]\nkey='k.pem'", "web.key"),
        ("[web]\nport=1", "web.port"),
    ] {
        std::fs::write(&user, source).unwrap();
        let error = Config::load_layers(Some(&user), root.path(), None).unwrap_err();
        assert_eq!(error.file, user, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
    }
}

#[test]
fn tui_diff_defaults_off_and_a_layer_turns_it_on() {
    let root = tempfile::tempdir().unwrap();
    assert!(!Config::load_layers(None, root.path(), None).unwrap().tui.diff);
    std::fs::write(root.path().join(".spec-viewer.toml"), "[tui]\ndiff = true\n").unwrap();
    assert!(Config::load_layers(None, root.path(), None).unwrap().tui.diff);
}

#[test]
fn tabs_default_to_plans_then_specs_and_a_layer_replaces_them() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    let tabs = |config: Config| {
        config
            .parse
            .tabs
            .into_iter()
            .map(|tab| (tab.name, tab.folder, tab.kinds, tab.companions))
            .collect::<Vec<_>>()
    };
    let strings = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        tabs(Config::default()),
        [
            ("plans".into(), Some(".plan".into()), None, strings(&["examples"])),
            ("specs".into(), None, Some(strings(&["spec"])), strings(&["examples", "discovery"])),
        ]
    );
    std::fs::write(
        &file,
        "[[tab]]\nname='ADRs'\nfolder='docs/adr'\n[[tab]]\nname='docs'\nfolder='docs'\nkinds=['guide']\n",
    )
    .unwrap();
    assert_eq!(
        tabs(Config::load_layers(None, root.path(), None).unwrap()),
        [
            ("ADRs".into(), Some("docs/adr".into()), None, vec![]),
            ("docs".into(), Some("docs".into()), Some(strings(&["guide"])), vec![]),
        ]
    );
}

#[test]
fn tab_of_picks_the_deepest_folder_and_its_kinds() {
    let parse = Config::default().parse;
    for (path, tab) in [
        (".plan/a.md", Some(0)),
        (".plan/a.spec.mdx", Some(0)),
        ("a.spec.md", Some(1)),
        ("dir/a.KD-1.discovery.mdx", Some(1)),
        ("README.md", None),
        ("a.spec.txt", None),
        (".planned/a.spec.md", Some(1)),
    ] {
        assert_eq!(parse.tab_of(path), tab, "{path}");
    }
}

#[test]
fn tab_errors_name_the_tab_and_key() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    let tab = |name: &str, folder: &str| format!("[[tab]]\nname='{name}'\nfolder='{folder}'\n");
    for (source, key) in [
        (tab(" ", "a"), "tab[0].name"),
        (tab("a", "a") + &tab("A", "b"), "tab[1].name"),
        (tab("a", ""), "tab[0].folder"),
        (tab("a", "/abs"), "tab[0].folder"),
        (tab("a", "../out"), "tab[0].folder"),
        (tab("a", ".plan/"), "tab[0].folder"),
        (tab("a", "docs") + &tab("b", "docs"), "tab[1].folder"),
        ("[[tab]]\nname='a'\n[[tab]]\nname='b'\n".into(), "tab[1].folder"),
        (tab("a", "a") + "kinds=[]\n", "tab[0].kinds"),
        (tab("a", "a") + "kinds=['spec.x']\n", "tab[0].kinds"),
        (tab("a", "a") + "kinds=['spec']\ncompanions=['spec']\n", "tab[0].companions"),
        ("[[tab]]\nfolder='a'".into(), "tab[0]"),
        ("tab = []".into(), "tab"),
        ((0..10).map(|i| tab(&i.to_string(), &i.to_string())).collect(), "tab"),
    ] {
        std::fs::write(&file, &source).unwrap();
        let error = Config::load_layers(None, root.path(), None).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
    }
}

#[test]
fn keys_parse_herdr_strings_and_a_layer_rebinds_one_action() {
    use spec_viewer_config::{Action, Code, Key, Modifiers};
    let root = tempfile::tempdir().unwrap();
    let plain = |code| Key::new(code, Modifiers::default());
    let ctrl = |character| {
        Key::new(
            Code::Char(character),
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        )
    };
    let keys = Config::load_layers(None, root.path(), None).unwrap().keys;
    assert_eq!(keys.action(&plain(Code::Char('q'))), Some((Action::Quit, 0)));
    assert_eq!(keys.action(&plain(Code::Char('Y'))), Some((Action::CopyName, 0)));
    assert_eq!(keys.action(&plain(Code::Char('3'))), Some((Action::SwitchTab, 2)));
    assert_eq!(keys.action(&ctrl('c')), Some((Action::ForceQuit, 0)));
    assert_eq!(
        keys.action(&Key::new(
            Code::Tab,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            }
        )),
        Some((Action::ToggleFocus, 0))
    );

    std::fs::write(
        root.path().join(".spec-viewer.toml"),
        "[keys]\nquit = ['x', 'ctrl+shift+q']\ncopy_name = 'shift+n'\nswitch_tab = 'alt+1..9'\nhints = []\n[keys.search_dialog]\nclose = ['esc', 'ctrl+plus']\n",
    )
    .unwrap();
    let keys = Config::load_layers(None, root.path(), None).unwrap().keys;
    assert_eq!(keys.action(&plain(Code::Char('x'))), Some((Action::Quit, 0)));
    assert_eq!(keys.action(&plain(Code::Char('q'))), None);
    assert_eq!(
        keys.action(&Key::new(
            Code::Char('Q'),
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            }
        )),
        Some((Action::Quit, 0))
    );
    assert_eq!(keys.action(&plain(Code::Char('N'))), Some((Action::CopyName, 0)));
    assert_eq!(keys.action(&plain(Code::Char('1'))), None);
    assert_eq!(
        keys.action(&Key::new(
            Code::Char('9'),
            Modifiers {
                alt: true,
                ..Modifiers::default()
            }
        )),
        Some((Action::SwitchTab, 8))
    );
    assert_eq!(keys.action(&plain(Code::Char('f'))), None);
    assert!(keys.search_dialog.close.matches(&ctrl('+')));
    assert_eq!(keys.copy_name.label().as_deref(), Some("N"));
    assert_eq!(keys.search_dialog.clear.label().as_deref(), Some("C-u"));
    assert_eq!(keys.toggle_focus.0[1].key.label(), "S-Tab");
    assert!(keys.down.matches(&plain(Code::Char('j'))), "other actions keep their defaults");
}

#[test]
fn key_errors_name_the_action() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    for (source, key, message) in [
        ("[keys]\nquit = 'hyper+q'", "keys.quit", "unknown modifier"),
        ("[keys]\nquit = 'ctrl+ctrl+q'", "keys.quit", "repeated modifier"),
        ("[keys]\nquit = 'shift+/'", "keys.quit", "shifted character"),
        ("[keys]\nquit = 'nope'", "keys.quit", "unknown key name"),
        ("[keys]\nquit = 3", "keys.quit", "expected a key string"),
        ("[keys]\nstrange = 'x'", "keys.strange", "unknown field"),
        ("[keys.search_dialog]\nstrange = 'x'", "keys.search_dialog.strange", "unknown field"),
        ("[keys]\nhints = 'd'", "keys.hints", "also bound to keys.diff"),
        ("[keys]\nquit = '3'", "keys.quit", "3 is also bound to keys.switch_tab"),
        ("[keys]\nswitch_tab = '1'", "keys.switch_tab", "1..9 range"),
        ("[keys]\nquit = '1..9'", "keys.quit", "only allowed in keys.switch_tab"),
        (
            "[keys.search_dialog]\nclear = 'ctrl+c'",
            "keys.search_dialog.clear",
            "also bound to keys.force_quit",
        ),
        (
            "[keys.hint_mode]\nclose = 'backspace'",
            "keys.hint_mode.close",
            "also bound to keys.hint_mode.delete_char",
        ),
    ] {
        std::fs::write(&file, source).unwrap();
        let error = Config::load_layers(None, root.path(), None).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
        assert!(error.to_string().contains(message), "{source}: {error}");
    }
}
