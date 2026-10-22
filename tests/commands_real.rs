#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use pill::tools::commands::{CommandError, CommandRunner, CommandSpec, CommandsConfig};
use tempfile::TempDir;

fn powershell() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH disponible"))
        .map(|directory| directory.join("pwsh.exe"))
        .find(|path| path.is_file())
        .expect("las pruebas reales requieren pwsh.exe instalado")
        .canonicalize()
        .unwrap()
}

struct Fixture {
    _temporary: TempDir,
    root: PathBuf,
    trust: PathBuf,
    config: CommandsConfig,
}

impl Fixture {
    fn new(script: &str) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("proyecto con ñ");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("acción.ps1"), script).unwrap();
        let trust = temporary.path().join("privado").join("trust.json");
        Self {
            _temporary: temporary,
            root,
            trust,
            config: CommandsConfig {
                timeout_ms: 10_000,
                poll_interval_ms: 10,
                max_output_bytes: 1024,
                max_watched_bytes: 128 * 1024 * 1024,
                commands: vec![CommandSpec {
                    id: "comprobar".into(),
                    program: powershell(),
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-ExecutionPolicy".into(),
                        "Bypass".into(),
                        "-File".into(),
                        "acción.ps1".into(),
                    ],
                    cwd: PathBuf::from("."),
                    watched_files: vec!["acción.ps1".into()],
                }],
            },
        }
    }

    fn runner(&self) -> CommandRunner {
        CommandRunner::new(&self.root, self.config.clone(), &self.trust).unwrap()
    }
    fn granted(&self) -> CommandRunner {
        let runner = self.runner();
        let proposal = runner.proposals().unwrap().remove(0);
        assert!(!proposal.granted);
        runner
            .grant(&proposal.spec.id, &proposal.fingerprint)
            .unwrap();
        runner
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn cancel_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn registered_command_requires_explicit_grant_and_executes_real_output() {
    let fixture = Fixture::new(
        "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Out.Write('salida real ñ'); [Console]::Error.Write('diagnóstico real'); [IO.File]::WriteAllText((Join-Path $PWD 'resultado.txt'),'hecho'); exit 0",
    );
    let runner = fixture.runner();
    assert!(
        !fixture.trust.parent().unwrap().exists(),
        "proponer no crea directorios privados"
    );
    assert!(matches!(
        runtime().block_on(runner.execute("comprobar", cancel_flag())),
        Err(CommandError::NotGranted(_))
    ));
    assert!(!fixture.root.join("resultado.txt").exists());
    let proposal = runner.proposals().unwrap().remove(0);
    runner.grant("comprobar", &proposal.fingerprint).unwrap();
    assert!(fixture.trust.is_file());
    let result = runtime()
        .block_on(runner.execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(
        result.exit_code,
        Some(0),
        "stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
    assert_eq!(result.stdout, "salida real ñ");
    assert_eq!(result.stderr, "diagnóstico real");
    assert_eq!(
        fs::read_to_string(fixture.root.join("resultado.txt")).unwrap(),
        "hecho"
    );
    assert!(!result.cancelled && !result.timed_out);
    runner.revoke("comprobar").unwrap();
    assert!(!runner.proposals().unwrap()[0].granted);
}

#[test]
fn nonzero_exit_and_output_limits_are_observed() {
    let mut fixture = Fixture::new(
        "[Console]::Out.Write(('a' * 20000)); [Console]::Error.Write(('b' * 30000)); exit 17",
    );
    fixture.config.max_output_bytes = 4096;
    let result = runtime()
        .block_on(fixture.granted().execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(result.exit_code, Some(17));
    assert_eq!(result.stdout, "a".repeat(4096));
    assert_eq!(result.stderr, "b".repeat(4096));
    assert!(result.stdout_truncated && result.stderr_truncated);
}

#[test]
fn invalid_utf8_is_reported_and_decoded_output_stays_bounded() {
    let mut fixture = Fixture::new("[Console]::OpenStandardOutput().Write([byte[]]@(255,255),0,2)");
    fixture.config.max_output_bytes = 3;
    let result = runtime()
        .block_on(fixture.granted().execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout_lossy && result.stdout_truncated);
    assert!(result.stdout.len() <= 3);
}

#[test]
fn changed_script_definition_and_stale_ui_proposal_require_new_consent() {
    let fixture = Fixture::new("[Console]::Out.Write('original')");
    let runner = fixture.granted();
    let old = runner.proposals().unwrap().remove(0);
    fs::write(
        fixture.root.join("acción.ps1"),
        "[Console]::Out.Write('modificado')",
    )
    .unwrap();
    assert!(!runner.proposals().unwrap()[0].granted);
    assert!(matches!(
        runner.grant("comprobar", &old.fingerprint),
        Err(CommandError::StaleProposal(_))
    ));
    assert!(matches!(
        runtime().block_on(runner.execute("comprobar", cancel_flag())),
        Err(CommandError::NotGranted(_))
    ));
    let proposal = runner.proposals().unwrap().remove(0);
    runner.grant("comprobar", &proposal.fingerprint).unwrap();
    let result = runtime()
        .block_on(runner.execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(result.stdout, "modificado");
    let mut different = fixture.config.clone();
    different.commands[0].args.push("argumento-nuevo".into());
    let runner = CommandRunner::new(&fixture.root, different, &fixture.trust).unwrap();
    assert!(!runner.proposals().unwrap()[0].granted);
}

#[test]
fn shell_metacharacters_remain_literal_arguments() {
    let mut fixture = Fixture::new(
        "param([string] $value) [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Out.Write($value)",
    );
    let argument = "á espacio \"cita\" & ; $(no-ejecutar) | > ruta\\";
    fixture.config.commands[0].args.push(argument.into());
    let result = runtime()
        .block_on(fixture.granted().execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, argument);
}

#[test]
fn command_scope_and_private_store_are_enforced() {
    let mut fixture = Fixture::new("exit 0");
    assert!(
        CommandRunner::new(
            &fixture.root,
            fixture.config.clone(),
            &fixture.root.join("trust.json")
        )
        .is_err()
    );
    fixture.config.commands[0].cwd = "..".into();
    assert!(CommandRunner::new(&fixture.root, fixture.config.clone(), &fixture.trust).is_err());
    fixture.config.commands[0].cwd = ".".into();
    fixture.config.commands[0].watched_files = vec!["../fuera.ps1".into()];
    assert!(CommandRunner::new(&fixture.root, fixture.config, &fixture.trust).is_err());
}

#[test]
fn cwd_requires_a_locked_observed_descendant() {
    let mut fixture = Fixture::new("exit 0");
    fixture.config.commands[0].args = vec![
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        "exit 0".into(),
    ];
    fixture.config.commands[0].watched_files.clear();
    assert!(fixture.runner().proposals().is_err());
}

#[test]
fn junctions_cannot_redirect_command_working_directory() {
    let mut fixture = Fixture::new("exit 0");
    let outside = fixture._temporary.path().join("fuera");
    fs::create_dir(&outside).unwrap();
    let junction = fixture.root.join("enlace");
    let output = Command::new(powershell())
        .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:PILL_TEST_JUNCTION -Target $env:PILL_TEST_TARGET -ErrorAction Stop | Out-Null"])
        .env("PILL_TEST_JUNCTION", &junction).env("PILL_TEST_TARGET", &outside)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.config.commands[0].cwd = "enlace".into();
    assert!(fixture.runner().proposals().is_err());
}

const TREE_SCRIPT: &str = r#"
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$child = Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"' -WindowStyle Hidden -PassThru
[IO.File]::WriteAllText((Join-Path $PWD 'child.pid'),[string]$child.Id)
[Console]::Out.Write('árbol iniciado')
Start-Sleep -Seconds 30
"#;

struct OtherChild(Child);
impl OtherChild {
    fn new() -> Self {
        use std::os::windows::process::CommandExt;
        Self(
            Command::new(powershell())
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 30",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
        )
    }
}
impl Drop for OtherChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "no apareció PID del hijo real");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_process_gone(pid: u32) {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        return;
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    assert_eq!(
        unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
        WAIT_OBJECT_0,
        "hijo {pid} continuó activo"
    );
}

#[test]
fn timeout_kills_only_the_owned_process_tree() {
    let mut fixture = Fixture::new(TREE_SCRIPT);
    fixture.config.timeout_ms = 5000;
    let runner = fixture.granted();
    let mut unrelated = OtherChild::new();
    let result = runtime()
        .block_on(runner.execute("comprobar", cancel_flag()))
        .unwrap();
    assert!(result.timed_out && !result.cancelled);
    assert!(result.stdout.contains("árbol iniciado"), "{result:?}");
    wait_process_gone(wait_pid(&fixture.root.join("child.pid")));
    assert!(
        unrelated.0.try_wait().unwrap().is_none(),
        "no se conserva proceso ajeno"
    );
}

#[test]
fn cancellation_kills_only_the_owned_process_tree() {
    let fixture = Fixture::new(TREE_SCRIPT);
    let runner = fixture.granted();
    let mut unrelated = OtherChild::new();
    let cancel = cancel_flag();
    let cancellation = cancel.clone();
    let pid_path = fixture.root.join("child.pid");
    let watcher = std::thread::spawn(move || {
        let pid = wait_pid(&pid_path);
        cancellation.store(true, Ordering::Release);
        pid
    });
    let result = runtime()
        .block_on(runner.execute("comprobar", cancel))
        .unwrap();
    assert!(result.cancelled && !result.timed_out, "{result:?}");
    wait_process_gone(watcher.join().unwrap());
    assert!(unrelated.0.try_wait().unwrap().is_none());
}

#[test]
fn scripts_are_pinned_against_external_write_during_execution() {
    let fixture = Fixture::new(
        "[IO.File]::WriteAllText((Join-Path $PWD 'started'),'sí'); Start-Sleep -Seconds 30",
    );
    let runner = fixture.granted();
    let cancel = cancel_flag();
    let cancelled = cancel.clone();
    let root = fixture.root.clone();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("started").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let write = fs::write(root.join("acción.ps1"), "exit 99");
        let remove = fs::remove_file(root.join("acción.ps1"));
        let rename = fs::rename(&root, root.with_file_name("directorio sustituido"));
        cancelled.store(true, Ordering::Release);
        assert!(
            write.is_err(),
            "escritura externa ganó entre hash y lectura real"
        );
        assert!(
            remove.is_err(),
            "archivo observado se pudo borrar en ejecución"
        );
        assert!(
            rename.is_err(),
            "directorio pinned se pudo sustituir en ejecución"
        );
    });
    let result = runtime()
        .block_on(runner.execute("comprobar", cancel))
        .unwrap();
    writer.join().unwrap();
    assert!(result.cancelled);
}

#[test]
fn parent_exit_also_cleans_up_its_remaining_descendants() {
    let fixture = Fixture::new(&TREE_SCRIPT.replace("Start-Sleep -Seconds 30\n", "exit 0\n"));
    let result = runtime()
        .block_on(fixture.granted().execute("comprobar", cancel_flag()))
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.timed_out);
    wait_process_gone(wait_pid(&fixture.root.join("child.pid")));
}
