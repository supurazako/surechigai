use std::process::Command;

#[test]
fn help_and_invalid_input_do_not_initialize_bluetooth() {
    let executable = env!("CARGO_BIN_EXE_surechigai");
    let help = Command::new(executable).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--rssi-threshold"));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--who"));

    for args in [
        vec!["--who".to_string(), "あ".repeat(22)],
        vec!["--who".into(), String::new()],
        vec!["--name".to_string(), "あ".repeat(11)],
        vec!["--name".into(), String::new()],
        vec!["--role-min-secs=0".into()],
        vec!["--rssi-threshold=127".into()],
    ] {
        let output = Command::new(executable).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(
            output.stdout.is_empty(),
            "must validate before initializing radio"
        );
    }
}
