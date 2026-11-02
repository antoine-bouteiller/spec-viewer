use spec_viewer_config::{Color, Config, HostConfig, Layer, Mode, ParseConfig};

#[test]
fn vc_7_user_tables_merge_and_project_layers_replace_arrays() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("user.toml");
    let worktree = root.path().join(".spec-viewer.toml");
    std::fs::write(
        &user,
        "[theme.roles]\naccent='red'\nheadings=['blue']\n[[status]]\nname='custom'\ncolor='red'\n",
    )
    .unwrap();
    std::fs::write(
        &worktree,
        "chip=[]\n[[status]]\nname='shipped'\naliases=['released']\ncolor='magenta'\n",
    )
    .unwrap();
    let config = Config::load_file(Some(&user)).unwrap();
    let theme = config.resolve(Mode::Dark).unwrap();
    assert_eq!(theme.roles.accent, Color::Ansi(spec_viewer_config::Ansi::Red));
    assert_eq!(theme.roles.headings.len(), 1);
    assert_eq!(config.parse.statuses[0].name, "custom");
    assert_eq!(config.layer.statuses.as_ref().map(Vec::len), Some(1));
    assert_eq!((config.layer.tabs.as_ref(), config.layer.chips.as_ref()), (None, None));
    let project = Layer::read(&worktree).unwrap().unwrap();
    let parse = ParseConfig::with(&[&config.layer, &project]);
    assert_eq!(parse.statuses.len(), 1);
    assert_eq!(parse.statuses[0].name, "shipped");
    assert!(parse.chips.is_empty());
    assert_eq!(parse.tabs, ParseConfig::default().tabs);
    assert_eq!(Layer::read(&root.path().join("absent.toml")).unwrap(), None);
}

#[test]
fn vc_8_worktree_files_hold_project_sections_only() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    for (source, key) in [
        ("[[host]]\nname='box'\nssh='box'", "host"),
        ("[web]\nport=1", "web"),
        ("[keys]\nquit='x'", "keys"),
        ("[theme]\nmode='dark'", "theme"),
        ("[tui]\ndiff=true", "tui"),
        ("strange=true", "strange"),
        ("[[tab]]\nname='a'\nfolder='a'\nstrange=1", "tab[0].strange"),
        (
            "[[status]]\nname='x'\ncolor='blue'\n[[status]]\nname='X'\ncolor='red'",
            "status[1].name",
        ),
        ("tab=[]", "tab"),
        ("[[tab]]\nname='a'\nfolder='../a'", "tab[0].folder"),
        ("[[tab]\n", "line 1"),
    ] {
        std::fs::write(&file, source).unwrap();
        let error = Layer::read(&file).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert_eq!(error.key, key, "{source}: {error}");
    }
    std::fs::write(&file, "[[status]]\nname='shipped'\ncolor='not-in-any-palette'").unwrap();
    assert_eq!(
        Layer::read(&file).unwrap().unwrap().statuses.unwrap()[0].color,
        "not-in-any-palette"
    );
}

#[test]
fn vc_8_user_web_takes_a_port_and_hosts_one_command_each() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("config.toml");
    assert_eq!(Config::default().web.port, 4747);
    assert!(Config::default().hosts.is_empty());
    std::fs::write(
        &user,
        "[web]\nport=0\n[[host]]\nname='box'\nssh='devbox'\n[[host]]\nname='pod'\ncommand=['kubectl','exec']\n",
    )
    .unwrap();
    let config = Config::load_file(Some(&user)).unwrap();
    assert_eq!(config.web.port, 0);
    assert_eq!(
        config.hosts.iter().map(HostConfig::argv).collect::<Vec<_>>(),
        [
            vec!["ssh", "-T", "devbox", "spec-viewer", "daemon", "--stdio"],
            vec!["kubectl", "exec"]
        ]
    );
    for (source, key) in [
        ("[web]\nbind='0.0.0.0'", "web.bind"),
        ("[web]\ncert='c.pem'", "web.cert"),
        ("[web]\nkey='k.pem'", "web.key"),
        ("[web]\nport=70000", "web.port"),
        ("[[host]]\nname='box'\nssh='box'\ncommand=['x']", "host[0].command"),
        ("[[host]]\nname='box'", "host[0]"),
        ("[[host]]\nname='box'\ncommand=[]", "host[0].command"),
        ("[[host]]\nname='box'\nssh='a'\n[[host]]\nname='box'\nssh='b'", "host[1].name"),
        ("[[host]]\nname='box'\nssh='a'\nport=22", "host[0].port"),
    ] {
        std::fs::write(&user, source).unwrap();
        let error = Config::load_file(Some(&user)).unwrap_err();
        assert_eq!(error.file, user, "{source}: {error}");
        assert_eq!(error.key, key, "{source}: {error}");
    }
}

#[test]
fn layers_cross_the_wire_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    std::fs::write(&file, "chip=[]\n[[tab]]\nname='notes'\nfolder='notes'\n").unwrap();
    let layer = Layer::read(&file).unwrap().unwrap();
    let json = serde_json::to_string(&layer).unwrap();
    assert_eq!(
        json,
        r#"{"tabs":[{"name":"notes","folder":"notes","kinds":null,"companions":[]}],"chips":[]}"#
    );
    assert_eq!(serde_json::from_str::<Layer>(&json).unwrap(), layer);
    assert_eq!(serde_json::from_str::<Layer>("{}").unwrap(), Layer::default());
}

#[test]
fn vc_7_validation_errors_name_the_defining_file_and_key() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.toml");
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
        let error = Config::load_file(Some(&file)).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
        assert_eq!(error.to_string().lines().count(), 1);
    }
    Config::load_file(Some(&root.path().join("absent-user.toml"))).unwrap();
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
        root.path().join("config.toml"),
        "[theme.palette]\nbrand='reset'\n[theme.roles]\naccent='brand'",
    )
    .unwrap();
    assert_eq!(
        Config::load_file(Some(&root.path().join("config.toml")))
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
    let file = root.path().join("config.toml");
    std::fs::write(&file, "[theme.unknown]\n").unwrap();
    let error = Config::load_file(Some(&root.path().join("config.toml"))).unwrap_err();
    assert_eq!(error.file, file);
    assert!(error.key.contains("unknown"));
}

#[test]
fn vc_7_palette_shape_errors_identify_the_exact_key_and_layer() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.toml");
    for (source, key) in [
        ("[theme.palette]\nblue=123", "theme.palette.blue"),
        ("[theme.palette.autumn]\nblue='#123456'", "theme.palette.autumn"),
        ("[theme.palette.light]\nblue=false", "theme.palette.light.blue"),
        ("[theme.palette]\nlight='blue'", "theme.palette.light"),
    ] {
        std::fs::write(&file, source).unwrap();
        let error = Config::load_file(Some(&root.path().join("config.toml"))).unwrap_err();
        assert_eq!(error.file, file);
        assert_eq!(error.key, key, "{source}: {error}");
    }
}

#[test]
fn tui_diff_defaults_off_and_a_layer_turns_it_on() {
    let root = tempfile::tempdir().unwrap();
    assert!(!Config::load_file(Some(&root.path().join("config.toml"))).unwrap().tui.diff);
    std::fs::write(root.path().join("config.toml"), "[tui]\ndiff = true\n").unwrap();
    assert!(Config::load_file(Some(&root.path().join("config.toml"))).unwrap().tui.diff);
}

#[test]
fn tabs_default_to_plans_then_specs_and_a_layer_replaces_them() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.toml");
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
        tabs(Config::load_file(Some(&root.path().join("config.toml"))).unwrap()),
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
    let file = root.path().join("config.toml");
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
        let error = Config::load_file(Some(&root.path().join("config.toml"))).unwrap_err();
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
    let keys = Config::load_file(Some(&root.path().join("config.toml"))).unwrap().keys;
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
        root.path().join("config.toml"),
        "[keys]\nquit = ['x', 'ctrl+shift+q']\ncopy_name = 'shift+n'\nswitch_tab = 'alt+1..9'\nhints = []\n[keys.search_dialog]\nclose = ['esc', 'ctrl+plus']\n",
    )
    .unwrap();
    let keys = Config::load_file(Some(&root.path().join("config.toml"))).unwrap().keys;
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
    let file = root.path().join("config.toml");
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
        let error = Config::load_file(Some(&root.path().join("config.toml"))).unwrap_err();
        assert_eq!(error.file, file, "{source}: {error}");
        assert!(error.key.starts_with(key), "{source}: {error}");
        assert!(error.to_string().contains(message), "{source}: {error}");
    }
}

#[test]
fn status_colors_resolve_against_this_palette_and_skip_unknown_names() {
    let status = |name: &str, color: &str| spec_viewer_config::StatusConfig {
        name: name.into(),
        aliases: vec![],
        color: color.into(),
    };
    let colors = Config::default().status_colors(Mode::Light, &[status("shipped", "green"), status("odd", "not-a-colour")]);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors["shipped"], Color::Ansi(spec_viewer_config::Ansi::Green));
}
