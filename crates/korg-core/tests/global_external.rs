//! Global field edits follow the runtime descriptor rather than baked offsets.

use korg_core::{ControllerAction, ControllerInput, EditorController, ManualClock};
use korg_midi::MockTransport;
use korg_model::{FileModelKey, GlobalData};

#[test]
fn changed_external_global_offsets_are_used_for_display_and_writes() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors/korg");
    let root = std::env::temp_dir().join(format!("korg-global-fields-{}", std::process::id()));
    let destination = root.join("korg");
    std::fs::create_dir_all(&destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "kdl") {
            std::fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
        }
    }
    let file = destination.join("nanokey-studio.global.inc.kdl");
    let original = std::fs::read_to_string(&file).unwrap();
    let changed = original
        .replace("name-offset=8", "name-offset=20")
        .replace("bpm-offset=57", "bpm-offset=70")
        .replace("bpm-min=200 bpm-max=3000", "bpm-min=1000 bpm-max=1500");
    assert_ne!(original, changed);
    std::fs::write(file, changed).unwrap();
    std::env::set_var("KORG_DESCRIPTOR_DIR", &root);

    let mut controller = EditorController::new(MockTransport::default(), ManualClock::new());
    controller.set_schema(korg_devices::parameter_schema("X14800", "global"));
    controller.load(ControllerInput::Global(GlobalData {
        model: FileModelKey::new("1480").unwrap(),
        data: vec![0; 128],
    }));
    assert!(controller
        .dispatch(ControllerAction::SetGlobalText {
            offset: 20,
            text: "Test".into()
        })
        .error
        .is_none());
    assert!(controller
        .dispatch(ControllerAction::SetGlobalBpm(1234))
        .error
        .is_none());
    assert!(controller
        .dispatch(ControllerAction::SetGlobalBpm(999))
        .error
        .is_some());
    assert!(controller
        .dispatch(ControllerAction::SetGlobalBpm(1501))
        .error
        .is_some());
    let global = &controller
        .session()
        .document()
        .global
        .as_ref()
        .unwrap()
        .data;
    assert_eq!(&global[20..24], b"Test");
    assert_eq!(&global[70..72], &1234u16.to_le_bytes());
    assert_eq!(&global[57..59], &[0, 0]);
    let row = controller
        .view()
        .global_params
        .into_iter()
        .find(|row| row.key == korg_core::DEVICE_NAME_KEY)
        .unwrap();
    assert_eq!(row.offset, 20);
    assert_eq!(row.display, "Test");
    std::fs::remove_dir_all(root).unwrap();
}
