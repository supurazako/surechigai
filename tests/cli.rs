use std::process::Command;

#[test]
fn help_and_invalid_input_do_not_initialize_bluetooth() {
    let executable = env!("CARGO_BIN_EXE_surechigai");
    let help = Command::new(executable).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--rssi-threshold"));

    for args in [
        vec!["--message".to_string(), "あ".repeat(43)],
        vec![
            "--message".into(),
            "test".into(),
            "--role-min-secs=0".into(),
        ],
        vec![
            "--message".into(),
            "test".into(),
            "--rssi-threshold=127".into(),
        ],
    ] {
        let output = Command::new(executable).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(
            output.stdout.is_empty(),
            "must validate before initializing radio"
        );
    }
}
