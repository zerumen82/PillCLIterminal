#![cfg(windows)]

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use pill::{
    config::Config,
    providers::ollama::{ToolCall, ToolFunction},
    tools::{Role, Toolset},
};
use serde_json::{Value, json};

struct Project {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    tools: Toolset,
    runtime: tokio::runtime::Runtime,
}

impl Project {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("proyecto ñ");
        fs::create_dir(&root).unwrap();
        let config: Config = serde_json::from_str(include_str!("../config/defaults.json")).unwrap();
        let tools = Toolset::new(
            &root,
            &config,
            None,
            &temporary.path().join("privado/trust.json"),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Self {
            _temporary: temporary,
            root,
            tools,
            runtime,
        }
    }

    fn execute(&self, role: Role, name: &str, arguments: Value) -> Value {
        self.runtime.block_on(self.tools.execute(
            role,
            &ToolCall {
                function: ToolFunction {
                    name: name.into(),
                    arguments,
                },
            },
            Arc::new(AtomicBool::new(false)),
        ))
    }

    fn inspected_sha(&self, name: &str, path: &str) -> String {
        let value = self.execute(Role::Explorer, name, json!({"path":path}));
        assert_eq!(value["status"], "verified", "{value}");
        value["sha256"].as_str().unwrap().into()
    }

    fn grant(&self, proposal: &Value) {
        assert_eq!(proposal["status"], "awaiting_confirmation", "{proposal}");
        let id = proposal["offer"].as_str().unwrap();
        let offers = self.tools.offers().unwrap();
        let offer = offers.iter().find(|offer| offer.id == id).unwrap();
        self.tools.grant_offer(id, &offer.fingerprint).unwrap();
    }
}

#[test]
fn moving_requires_human_grant_and_consumes_it_even_if_same_object_returns() {
    let project = Project::new();
    let original = [0, 255, 1, 2];
    fs::write(project.root.join("base.bin"), original).unwrap();
    let sha = project.inspected_sha("inspect_path", "base.bin");
    let arguments = json!({"path":"base.bin", "destination":"nuevo.bin", "expected_sha256":sha});
    let proposal = project.execute(Role::Implementer, "move_file", arguments.clone());
    assert_eq!(proposal["status"], "awaiting_confirmation", "{proposal}");
    assert_eq!(fs::read(project.root.join("base.bin")).unwrap(), original);
    assert!(!project.root.join("nuevo.bin").exists());

    // Repetir la propuesta desde el modelo no equivale a concederla.
    let repeated = project.execute(Role::Implementer, "move_file", arguments.clone());
    assert_eq!(repeated["status"], "awaiting_confirmation", "{repeated}");
    assert!(!project.root.join("nuevo.bin").exists());
    project.grant(&proposal);
    let applied = project.execute(Role::Implementer, "move_file", arguments.clone());
    assert_eq!(applied["status"], "verified", "{applied}");
    assert!(!project.root.join("base.bin").exists());
    assert_eq!(fs::read(project.root.join("nuevo.bin")).unwrap(), original);
    project.tools.verify_versions().unwrap();

    // Restaurar solo el nombre de la muestra conserva identidad y bytes: la
    // segunda petición debe requerir otra concesión, aunque su huella coincida.
    fs::rename(
        project.root.join("nuevo.bin"),
        project.root.join("base.bin"),
    )
    .unwrap();
    assert_eq!(project.inspected_sha("inspect_path", "base.bin"), sha);
    let second = project.execute(Role::Implementer, "move_file", arguments);
    assert_eq!(second["status"], "awaiting_confirmation", "{second}");
    assert_eq!(fs::read(project.root.join("base.bin")).unwrap(), original);
    assert!(!project.root.join("nuevo.bin").exists());
}

#[test]
fn metadata_inspection_does_not_authorize_editing_without_reading_text() {
    let project = Project::new();
    fs::write(project.root.join("codigo.txt"), "original").unwrap();
    let sha = project.inspected_sha("inspect_path", "codigo.txt");
    let args = json!({"path":"codigo.txt","expected_sha256":sha,"old":"original","new":"editado"});
    assert_eq!(
        project.execute(Role::Implementer, "edit_file", args.clone())["status"],
        "failed"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("codigo.txt")).unwrap(),
        "original"
    );
    project.inspected_sha("read_file", "codigo.txt");
    assert_eq!(
        project.execute(Role::Implementer, "edit_file", args)["status"],
        "verified"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("codigo.txt")).unwrap(),
        "editado"
    );
}

#[test]
fn file_delete_requires_read_and_denied_roles_cannot_consume_human_grant() {
    let project = Project::new();
    fs::write(project.root.join("borrar.txt"), "solo esta muestra").unwrap();
    let observed = project.tools.files.inspect_operation("borrar.txt").unwrap();
    let arguments = json!({"path":"borrar.txt", "expected_sha256":observed["sha256"]});
    let unread = project.execute(Role::Implementer, "delete_file", arguments.clone());
    assert_eq!(unread["status"], "failed", "{unread}");
    assert!(project.root.join("borrar.txt").exists());

    let sha = project.inspected_sha("read_file", "borrar.txt");
    assert_eq!(observed["sha256"], sha);
    let proposal = project.execute(Role::Implementer, "delete_file", arguments.clone());
    project.grant(&proposal);
    for role in [
        Role::Explorer,
        Role::Tester,
        Role::Reviewer,
        Role::Coordinator,
    ] {
        let denied = project.execute(role, "delete_file", arguments.clone());
        assert_eq!(denied["status"], "failed", "{role:?}: {denied}");
        assert_eq!(
            fs::read(project.root.join("borrar.txt")).unwrap(),
            b"solo esta muestra"
        );
    }
    let applied = project.execute(Role::Implementer, "delete_file", arguments);
    assert_eq!(applied["status"], "verified", "{applied}");
    assert!(!project.root.join("borrar.txt").exists());
}

#[test]
fn an_empty_directory_is_inspected_and_only_the_confirmed_directory_is_removed() {
    let project = Project::new();
    fs::create_dir(project.root.join("vacía")).unwrap();
    fs::create_dir(project.root.join("otra")).unwrap();
    let sha = project.inspected_sha("inspect_path", "vacía");
    let arguments = json!({"path":"vacía", "expected_sha256":sha});
    let proposal = project.execute(Role::Implementer, "delete_file", arguments.clone());
    project.grant(&proposal);
    let forgery = project.execute(
        Role::Implementer,
        "delete_file",
        json!({"path":"otra", "expected_sha256":sha}),
    );
    assert_eq!(forgery["status"], "failed", "{forgery}");
    assert!(project.root.join("otra").is_dir());
    assert!(project.root.join("vacía").is_dir());
    let deleted = project.execute(Role::Implementer, "delete_file", arguments);
    assert_eq!(deleted["status"], "verified", "{deleted}");
    assert!(!project.root.join("vacía").exists());
    assert!(project.root.join("otra").is_dir());
}

#[test]
fn changed_base_invalidates_displayed_and_already_granted_operations() {
    let project = Project::new();
    fs::write(project.root.join("archivo"), "base").unwrap();
    let sha = project.inspected_sha("read_file", "archivo");
    let arguments = json!({"path":"archivo", "expected_sha256":sha});
    let proposal = project.execute(Role::Implementer, "delete_file", arguments.clone());
    assert_eq!(proposal["status"], "awaiting_confirmation", "{proposal}");
    let id = proposal["offer"].as_str().unwrap();
    let offers = project.tools.offers().unwrap();
    let fingerprint = offers
        .iter()
        .find(|offer| offer.id == id)
        .unwrap()
        .fingerprint
        .clone();
    fs::write(project.root.join("archivo"), "cambio del usuario").unwrap();
    assert!(project.tools.grant_offer(id, &fingerprint).is_err());
    let stale = project.execute(Role::Implementer, "delete_file", arguments);
    assert_eq!(stale["status"], "failed", "{stale}");
    assert_eq!(
        fs::read(project.root.join("archivo")).unwrap(),
        b"cambio del usuario"
    );

    let sha = project.inspected_sha("read_file", "archivo");
    let arguments = json!({"path":"archivo", "expected_sha256":sha});
    let proposal = project.execute(Role::Implementer, "delete_file", arguments.clone());
    project.grant(&proposal);
    fs::write(project.root.join("archivo"), "otro cambio").unwrap();
    let stale = project.execute(Role::Implementer, "delete_file", arguments);
    assert_eq!(stale["status"], "failed", "{stale}");
    assert_eq!(
        fs::read(project.root.join("archivo")).unwrap(),
        b"otro cambio"
    );
}
