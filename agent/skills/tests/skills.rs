use std::io::Write;
use std::path::Path;

use agenium_skills::*;

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn skill(dir: &Path, name: &str, desc: &str, body: &str) {
    write(
        &dir.join(name).join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n"),
    );
}

#[test]
fn frontmatter_subset() {
    let (fm, body) = parse(
        "---\nname: pdf-tools\ndescription: >\n  Work with PDF files.\n  Use it for forms.\nversion: \"1.2\"\nmetadata:\n  author: x\nlicense: 'Apache-2.0'\n---\n# Body\ntext",
    )
    .unwrap();
    assert_eq!(fm.get("name"), Some("pdf-tools"));
    assert_eq!(
        fm.get("description"),
        Some("Work with PDF files. Use it for forms.")
    );
    assert_eq!(fm.get("version"), Some("1.2"));
    assert_eq!(fm.get("license"), Some("Apache-2.0"));
    assert_eq!(fm.get("author"), None, "nested keys are ignored");
    assert_eq!(body, "# Body\ntext");
    assert!(parse("# no frontmatter").is_err());
    assert!(parse("---\nname: x\n").is_err(), "unclosed");
}

#[test]
fn names_and_references() {
    for ok in ["a", "pdf-tools", "x9", "owasp-top10"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in ["", "-a", "a-", "a--b", "PDF", "a_b", "a b", &"x".repeat(65)] {
        assert!(!valid_name(bad), "{bad}");
    }
    let r = references(
        "See [forms](references/forms.md#top) and [site](https://x.dev). Run `scripts/fill.py` then `cargo test`.\n![img](assets/a.png) [anchor](#x)",
    );
    assert_eq!(
        r,
        vec!["assets/a.png", "references/forms.md", "scripts/fill.py"]
    );
}

#[test]
fn validation_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    skill(
        d,
        "good",
        "Does good things.",
        "Read [ref](references/r.md).",
    );
    write(&d.join("good/references/r.md"), "ref");
    write(&d.join("good/scripts/run.sh"), "echo");
    let s = validate(&d.join("good"), Origin::Role).unwrap();
    assert_eq!(s.meta.name, "good");
    assert_eq!(s.files, vec!["references/r.md", "scripts/run.sh"]);
    assert!(s.instructions().contains("Read [ref]"));

    skill(d, "missing-ref", "x", "Use `scripts/nope.py`.");
    let e = validate(&d.join("missing-ref"), Origin::Role)
        .unwrap_err()
        .to_string();
    assert!(e.contains("does not exist"), "{e}");

    skill(d, "escape", "x", "See [secret](../good/references/r.md).");
    let e = validate(&d.join("escape"), Origin::Role)
        .unwrap_err()
        .to_string();
    assert!(e.contains("leaves the skill folder"), "{e}");

    skill(d, "abs", "x", "See [p](/etc/passwd).");
    assert!(validate(&d.join("abs"), Origin::Role).is_err());

    write(
        &d.join("wrong-dir/SKILL.md"),
        "---\nname: other\ndescription: x\n---\nbody",
    );
    let e = validate(&d.join("wrong-dir"), Origin::Role)
        .unwrap_err()
        .to_string();
    assert!(e.contains("folder name"), "{e}");

    write(&d.join("nodesc/SKILL.md"), "---\nname: nodesc\n---\nbody");
    assert!(validate(&d.join("nodesc"), Origin::Role).is_err());

    write(
        &d.join("nobody/SKILL.md"),
        "---\nname: nobody\ndescription: d\n---\n  \n",
    );
    assert!(validate(&d.join("nobody"), Origin::Role).is_err());
}

#[test]
fn tags() {
    // Comma- or space-separated, lowercased, deduplicated, order kept.
    assert_eq!(
        parse_tags("Pentest, reverse-engineering  pentest"),
        ["pentest", "reverse-engineering"]
    );
    assert_eq!(parse_tags(""), Vec::<String>::new());

    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    write(
        &d.join("tagged/SKILL.md"),
        "---\nname: tagged\ndescription: d\ntags: pentest, firmware\n---\nbody",
    );
    let s = validate(&d.join("tagged"), Origin::Role).unwrap();
    assert_eq!(s.meta.tags, ["pentest", "firmware"]);

    // A skill with no tags is fine.
    skill(d, "untagged", "d", "body");
    assert!(validate(&d.join("untagged"), Origin::Role)
        .unwrap()
        .meta
        .tags
        .is_empty());

    // An invalid tag slug is rejected.
    write(
        &d.join("badtag/SKILL.md"),
        "---\nname: badtag\ndescription: d\ntags: Pen Test!\n---\nbody",
    );
    assert!(validate(&d.join("badtag"), Origin::Role).is_err());
}

#[test]
fn write_create_and_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = Registry::new(tmp.path().join("agents"), tmp.path().join("roles"));
    let lib = Owner::Library;
    let good =
        "---\nname: shaper\ndescription: Shape a thing.\ntags: coding\n---\n## Job\nShape it.";

    // Create writes the folder from the text.
    let m = reg.write(&lib, "shaper", good, true).unwrap();
    assert_eq!(m.name, "shaper");
    assert_eq!(m.tags, ["coding"]);
    assert!(reg.dir_of(&lib).join("shaper/SKILL.md").is_file());

    // Create again is refused (already exists); editing is not.
    assert!(reg.write(&lib, "shaper", good, true).is_err());
    let edited = "---\nname: shaper\ndescription: Shape a thing well.\ntags: coding, craft\n---\n## Job\nShape it well.";
    let m2 = reg.write(&lib, "shaper", edited, false).unwrap();
    assert_eq!(m2.tags, ["coding", "craft"]);
    assert_eq!(
        validate(&reg.dir_of(&lib).join("shaper"), Origin::Library)
            .unwrap()
            .body
            .trim(),
        "## Job\nShape it well."
    );

    // The frontmatter name must equal the target name.
    assert!(reg
        .write(
            &lib,
            "shaper",
            "---\nname: other\ndescription: d\n---\n## Job\nx",
            false
        )
        .is_err());
    // Invalid content does not overwrite the good file.
    assert!(reg.write(&lib, "shaper", "no frontmatter", false).is_err());
    assert!(reg.dir_of(&lib).join("shaper/SKILL.md").is_file());
    assert!(validate(&reg.dir_of(&lib).join("shaper"), Origin::Library)
        .unwrap()
        .body
        .contains("well"));
    // Editing a skill that does not exist is refused.
    assert!(reg
        .write(
            &lib,
            "ghost",
            good.replace("shaper", "ghost").as_str(),
            false
        )
        .is_err());
}

#[test]
fn registry_resolution_and_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = Registry::new(tmp.path().join("agents"), tmp.path().join("roles"));
    let role = reg.dir_of(&Owner::Role("coder".into()));
    let own = reg.dir_of(&Owner::Agent("A1".into()));
    skill(&role, "rust-style", "Rust style rules.", "Role version.");
    skill(&role, "git-flow", "Branch rules.", "Role git.");
    skill(
        &own,
        "rust-style",
        "Rust style rules (learned).",
        "Agent overlay.",
    );
    skill(&own, "my-notes", "Personal notes.", "Mine.");
    write(&own.join("broken/SKILL.md"), "no frontmatter");

    let set = reg.for_agent("A1", "coder", &[]);
    assert_eq!(set.names(), vec!["git-flow", "my-notes", "rust-style"]);
    let rs = set.get("rust-style").unwrap();
    assert_eq!(rs.meta.origin, Origin::Overlay);
    assert_eq!(rs.body.trim(), "Agent overlay.");
    assert_eq!(set.problems.len(), 1);
    let idx = set.index();
    assert!(
        idx.contains("- rust-style [updated]: Rust style rules (learned)."),
        "{idx}"
    );
    assert!(idx.contains("load_skill"));

    // Another agent of the same role sees the base, not A1's overlay.
    let other = reg.for_agent("A2", "coder", &[]);
    assert_eq!(
        other.get("rust-style").unwrap().body.trim(),
        "Role version."
    );
    assert_eq!(other.get("rust-style").unwrap().meta.origin, Origin::Role);

    // Library skills reach only the agents that enable them; a role skill of the same name wins.
    let lib = reg.dir_of(&Owner::Library);
    assert_eq!(lib, tmp.path().join("skills"));
    skill(&lib, "pdf-reading", "Read PDFs.", "Library pdf.");
    skill(&lib, "git-flow", "Branch rules (library).", "Library git.");
    assert_eq!(reg.library().names(), vec!["git-flow", "pdf-reading"]);
    assert_eq!(
        reg.for_agent("A2", "coder", &[]).names(),
        vec!["git-flow", "rust-style"]
    );
    let enabled = reg.for_agent("A2", "coder", &["pdf-reading".into(), "git-flow".into()]);
    assert_eq!(
        enabled.names(),
        vec!["git-flow", "pdf-reading", "rust-style"]
    );
    assert_eq!(
        enabled.get("pdf-reading").unwrap().meta.origin,
        Origin::Library
    );
    assert_eq!(enabled.get("git-flow").unwrap().body.trim(), "Role git.");
    assert_eq!(
        SkillSet::default().index(),
        "(no skills are installed for you)"
    );
}

#[test]
fn a_plugin_is_on_whole_and_its_skills_stay_the_vendors() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = Registry::new(tmp.path().join("agents"), tmp.path().join("roles"));
    let plug = reg.plugins_dir().join("superpowers").join("skills");
    assert_eq!(reg.plugins_dir(), tmp.path().join("plugins"));
    skill(&plug, "brainstorming", "Brainstorm first.", "Vendor text.");
    skill(&plug, "git-flow", "Branch rules (plugin).", "Plugin git.");
    let role = reg.dir_of(&Owner::Role("coder".into()));
    skill(&role, "git-flow", "Branch rules.", "Role git.");
    let own = reg.dir_of(&Owner::Agent("A1".into()));
    skill(&own, "brainstorming", "Brainstorm (mine).", "Agent copy.");

    // Off: nothing of the plugin.
    assert_eq!(
        reg.for_agent("A1", "coder", &[]).names(),
        vec!["brainstorming", "git-flow"]
    );
    assert_eq!(
        reg.for_agent("A1", "coder", &[])
            .get("brainstorming")
            .unwrap()
            .meta
            .origin,
        Origin::Agent
    );
    // On: the plugin's skills, not over the role's, and the agent's own copy does not replace the vendor's.
    let on = reg.for_agent("A1", "coder", &["plugin:superpowers".into()]);
    assert_eq!(on.names(), vec!["brainstorming", "git-flow"]);
    let b = on.get("brainstorming").unwrap();
    assert_eq!(
        (b.meta.origin, b.body.trim()),
        (Origin::Plugin, "Vendor text.")
    );
    assert_eq!(on.get("git-flow").unwrap().body.trim(), "Role git.");
    assert!(
        on.problems.iter().any(|p| p.error.contains("read-only")),
        "{:?}",
        on.problems
    );
    // A plugin's skill may point at a sibling skill's file, never out of the plugin's skills folder.
    write(&plug.join("reviewing/notes.md"), "notes");
    skill(
        &plug,
        "planning",
        "Plan.",
        "See [notes](../reviewing/notes.md).",
    );
    write(
        &plug.join("reviewing/SKILL.md"),
        "---\nname: reviewing\ndescription: Review.\n---\nReview.\n",
    );
    assert!(validate(&plug.join("planning"), Origin::Plugin).is_ok());
    assert!(validate(&plug.join("planning"), Origin::Library).is_err());
    skill(&plug, "escaping", "Out.", "See [x](../../../secret.md).");
    assert!(validate(&plug.join("escaping"), Origin::Plugin).is_err());
    assert_eq!(plugin_of("plugin:superpowers"), Some("superpowers"));
    assert_eq!(plugin_of("plugin:../x"), None);
    assert_eq!(plugin_of("git-flow"), None);
}

#[test]
fn import_folder_and_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = Registry::new(tmp.path().join("agents"), tmp.path().join("roles"));
    let src = tmp.path().join("src");
    // Folder name differs from the skill name: the imported folder is named after the skill.
    write(
        &src.join("download/SKILL.md"),
        "---\nname: csv-report\ndescription: CSV reports.\n---\nUse `scripts/sum.py`.",
    );
    write(&src.join("download/scripts/sum.py"), "print(1)");
    let owner = Owner::Agent("A1".into());
    let m = reg.import(&src.join("download"), &owner, false).unwrap();
    assert_eq!(m.name, "csv-report");
    assert!(reg
        .dir_of(&owner)
        .join("csv-report/scripts/sum.py")
        .is_file());
    assert!(matches!(
        reg.import(&src.join("download"), &owner, false),
        Err(SkillError::Exists(_))
    ));
    reg.import(&src.join("download"), &owner, true).unwrap();

    // zip with a top folder
    let zpath = tmp.path().join("s.zip");
    {
        let f = std::fs::File::create(&zpath).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("pack/SKILL.md", o).unwrap();
        z.write_all(b"---\nname: zipped\ndescription: From a zip.\n---\nSee [r](references/r.md).")
            .unwrap();
        z.start_file("pack/references/r.md", o).unwrap();
        z.write_all(b"ref").unwrap();
        z.finish().unwrap();
    }
    let m = reg
        .import(&zpath, &Owner::Role("coder".into()), false)
        .unwrap();
    assert_eq!(m.name, "zipped");
    assert_eq!(m.origin, Origin::Role);

    // zip slip is refused
    let evil = tmp.path().join("evil.zip");
    {
        let f = std::fs::File::create(&evil).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("../../outside.txt", o).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
    }
    assert!(reg.import(&evil, &owner, false).is_err());
    assert!(!tmp.path().join("outside.txt").exists());

    // invalid skill leaves nothing behind
    write(
        &src.join("bad/SKILL.md"),
        "---\nname: bad\ndescription: d\n---\nUse `scripts/missing.sh`.",
    );
    assert!(reg.import(&src.join("bad"), &owner, false).is_err());
    assert!(!reg.dir_of(&owner).join("bad").exists());
    let left: Vec<_> = std::fs::read_dir(reg.dir_of(&owner))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 1, "only csv-report remains: {left:?}");

    reg.remove(&owner, "csv-report").unwrap();
    assert!(reg.remove(&owner, "csv-report").is_err());
}
