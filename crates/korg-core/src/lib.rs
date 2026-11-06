//! Platform-independent editor session.
//!
//! The UI sends `Action` values.  The session returns `Outcome` values.  There
//! is no platform I/O.  Time comes from an injected `Clock`.

use korg_model::{GlobalData, SceneSet};
use thiserror::Error;

pub mod controller;
pub mod global_policy;
pub mod measured;
pub mod transfer;
pub use controller::*;
pub use measured::*;
pub use transfer::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub scene_set: Option<SceneSet>,
    pub global: Option<GlobalData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    LoadSceneSet(SceneSet),
    LoadGlobal(GlobalData),
    SelectScene(usize),
    SetSceneData {
        index: usize,
        data: Vec<u8>,
    },
    SetGlobalData(Vec<u8>),
    MarkSaved,
    /// The device now holds this part of the document: it was just
    /// received, or written and read back identical.
    MarkReceived(ReceivedPart),
    /// Put back what the device last held, as one undo step.
    RevertToReceived,
    Undo,
    Redo,
}

/// The part of the document a device transfer covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceivedPart {
    SceneSet,
    Scene(usize),
    Global,
}

/// What the device last held, per scene and for the global block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Received {
    /// The model and element size the scenes belong to.
    model: Option<(korg_model::FileModelKey, u16)>,
    scenes: Vec<Option<Vec<u8>>>,
    global: Option<GlobalData>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub document_changed: bool,
    pub selection_changed: bool,
    pub dirty_changed: bool,
    pub error: Option<CoreError>,
}

impl Outcome {
    fn ok() -> Self {
        Self::default()
    }

    fn error(error: CoreError) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("no scene set is loaded")]
    NoSceneSet,
    #[error("no global data is loaded")]
    NoGlobalData,
    #[error("scene index {0} is out of range")]
    SceneIndex(usize),
    #[error("scene size {actual} does not match the element size {expected}")]
    SceneSize { expected: usize, actual: usize },
}

#[derive(Debug, Clone, Default)]
pub struct EditorSession {
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    selected_scene: usize,
    received: Received,
}

impl EditorSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn selected_scene(&self) -> usize {
        self.selected_scene
    }

    pub fn is_dirty(&self) -> bool {
        self.document != self.saved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// True when `RevertToReceived` would change the document.
    pub fn can_revert(&self) -> bool {
        self.reverted() != self.document
    }

    /// The document with every part the device last held put back.  Scenes
    /// go back only into a set of the same model and size; global data
    /// only over global data of the same model, or none.
    fn reverted(&self) -> Document {
        let mut document = self.document.clone();
        if let (Some(set), Some((model, size))) =
            (document.scene_set.as_mut(), self.received.model.as_ref())
        {
            if set.model == *model && set.element_size == *size {
                for (index, data) in self.received.scenes.iter().enumerate() {
                    if let (Some(data), Some(scene)) = (data, set.scenes.get_mut(index)) {
                        scene.name = korg_model::scene_name(&set.model, data);
                        scene.data = data.clone();
                    }
                }
            }
        }
        if let Some(global) = &self.received.global {
            if document
                .global
                .as_ref()
                .is_none_or(|current| current.model == global.model)
            {
                document.global = Some(global.clone());
            }
        }
        document
    }

    fn mark_received(&mut self, part: ReceivedPart) {
        match part {
            ReceivedPart::Global => self.received.global = self.document.global.clone(),
            ReceivedPart::SceneSet | ReceivedPart::Scene(_) => {
                let Some(set) = self.document.scene_set.as_ref() else {
                    return;
                };
                let model = Some((set.model.clone(), set.element_size));
                if self.received.model != model || part == ReceivedPart::SceneSet {
                    self.received.model = model;
                    self.received.scenes = vec![None; set.scenes.len()];
                }
                self.received.scenes.resize(set.scenes.len(), None);
                for (index, scene) in set.scenes.iter().enumerate() {
                    if part == ReceivedPart::SceneSet || part == ReceivedPart::Scene(index) {
                        self.received.scenes[index] = Some(scene.data.clone());
                    }
                }
            }
        }
    }

    pub fn dispatch(&mut self, action: Action) -> Outcome {
        let was_dirty = self.is_dirty();
        let mut outcome = Outcome::ok();
        match action {
            Action::LoadSceneSet(scene_set) => {
                self.load_scene_set(scene_set);
                outcome.document_changed = true;
                outcome.selection_changed = true;
            }
            Action::LoadGlobal(global) => {
                self.load_global(global);
                outcome.document_changed = true;
            }
            Action::SelectScene(index) => {
                let count = self
                    .document
                    .scene_set
                    .as_ref()
                    .map(|set| set.scenes.len())
                    .unwrap_or(0);
                if index >= count {
                    return Outcome::error(CoreError::SceneIndex(index));
                }
                if index != self.selected_scene {
                    self.selected_scene = index;
                    outcome.selection_changed = true;
                }
            }
            Action::SetSceneData { index, data } => {
                let valid = self
                    .document
                    .scene_set
                    .as_ref()
                    .map(|set| index < set.scenes.len())
                    .unwrap_or(false);
                if self.document.scene_set.is_none() {
                    return Outcome::error(CoreError::NoSceneSet);
                }
                if !valid {
                    return Outcome::error(CoreError::SceneIndex(index));
                }
                let expected = self
                    .document
                    .scene_set
                    .as_ref()
                    .map(|set| set.element_size as usize)
                    .unwrap_or(0);
                if data.len() != expected {
                    return Outcome::error(CoreError::SceneSize {
                        expected,
                        actual: data.len(),
                    });
                }
                let scene = &mut self.document.scene_set.as_mut().unwrap().scenes[index];
                // An edit that changes nothing must not leave an undo step.
                if scene.data == data {
                    return outcome;
                }
                self.snapshot();
                let set = self.document.scene_set.as_mut().unwrap();
                // The name lives in the data (the same rule named the scene
                // when it was loaded); keep the label in step with it.
                set.scenes[index].name = korg_model::scene_name(&set.model, &data);
                set.scenes[index].data = data;
                outcome.document_changed = true;
            }
            Action::SetGlobalData(data) => {
                if self.document.global.is_none() {
                    return Outcome::error(CoreError::NoGlobalData);
                }
                if self
                    .document
                    .global
                    .as_ref()
                    .is_some_and(|g| g.data == data)
                {
                    return outcome;
                }
                self.snapshot();
                self.document.global.as_mut().unwrap().data = data;
                outcome.document_changed = true;
            }
            Action::MarkSaved => {
                self.saved = self.document.clone();
            }
            Action::MarkReceived(part) => self.mark_received(part),
            Action::RevertToReceived => {
                let reverted = self.reverted();
                if reverted != self.document {
                    self.snapshot();
                    self.document = reverted;
                    outcome.document_changed = true;
                }
            }
            Action::Undo => {
                if let Some(previous) = self.undo.pop() {
                    self.redo
                        .push(std::mem::replace(&mut self.document, previous));
                    outcome.document_changed = true;
                }
            }
            Action::Redo => {
                if let Some(next) = self.redo.pop() {
                    self.undo.push(std::mem::replace(&mut self.document, next));
                    outcome.document_changed = true;
                }
            }
        }
        outcome.dirty_changed = was_dirty != self.is_dirty();
        outcome
    }

    fn load_scene_set(&mut self, scene_set: SceneSet) {
        self.document.scene_set = Some(scene_set);
        self.saved.scene_set = self.document.scene_set.clone();
        self.undo.clear();
        self.redo.clear();
        self.selected_scene = 0;
    }

    fn load_global(&mut self, global: GlobalData) {
        self.document.global = Some(global);
        self.saved.global = self.document.global.clone();
        self.undo.clear();
        self.redo.clear();
    }

    fn snapshot(&mut self) {
        self.undo.push(self.document.clone());
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_model::{FileModelKey, Scene, SceneSet};

    fn scene_set() -> SceneSet {
        SceneSet {
            model: FileModelKey::new("0820").unwrap(),
            element_size: 4,
            scenes: vec![
                Scene {
                    name: None,
                    data: vec![1, 2, 3, 4],
                },
                Scene {
                    name: None,
                    data: vec![5, 6, 7, 8],
                },
            ],
        }
    }

    #[test]
    fn reverts_to_what_the_device_last_held() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        assert!(!session.can_revert(), "nothing received yet");
        session.dispatch(Action::MarkReceived(ReceivedPart::SceneSet));
        session.dispatch(Action::SetSceneData {
            index: 0,
            data: vec![9, 9, 9, 9],
        });
        session.dispatch(Action::SetSceneData {
            index: 1,
            data: vec![7, 7, 7, 7],
        });
        assert!(session.can_revert());
        // Scene 2 was written and read back: it is what the device holds now.
        session.dispatch(Action::MarkReceived(ReceivedPart::Scene(1)));
        let outcome = session.dispatch(Action::RevertToReceived);
        assert!(outcome.document_changed);
        let scenes = &session.document().scene_set.as_ref().unwrap().scenes;
        assert_eq!(scenes[0].data, [1, 2, 3, 4]);
        assert_eq!(scenes[1].data, [7, 7, 7, 7]);
        assert!(!session.can_revert());
        // One undo step brings the edit back.
        session.dispatch(Action::Undo);
        assert_eq!(
            session.document().scene_set.as_ref().unwrap().scenes[0].data,
            [9, 9, 9, 9]
        );
    }

    #[test]
    fn revert_leaves_a_set_of_another_model_alone() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        session.dispatch(Action::MarkReceived(ReceivedPart::SceneSet));
        let mut other = scene_set();
        other.model = FileModelKey::new("0830").unwrap();
        other.scenes[0].data = vec![0; 4];
        session.dispatch(Action::LoadSceneSet(other));
        assert!(!session.can_revert());
        assert!(!session.dispatch(Action::RevertToReceived).document_changed);
    }

    #[test]
    fn an_edit_that_changes_nothing_leaves_no_undo_step() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        let same = session.document().scene_set.as_ref().unwrap().scenes[1]
            .data
            .clone();
        let outcome = session.dispatch(Action::SetSceneData {
            index: 1,
            data: same,
        });
        assert!(!outcome.document_changed);
        assert!(!session.can_undo());
        assert!(!session.is_dirty());
    }

    #[test]
    fn edit_undo_redo_and_save() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        assert!(!session.is_dirty());

        session.dispatch(Action::SetSceneData {
            index: 1,
            data: vec![9, 9, 9, 9],
        });
        assert!(session.is_dirty());
        assert_eq!(
            session.document().scene_set.as_ref().unwrap().scenes[1].data,
            vec![9, 9, 9, 9]
        );

        session.dispatch(Action::Undo);
        assert_eq!(
            session.document().scene_set.as_ref().unwrap().scenes[1].data,
            vec![5, 6, 7, 8]
        );
        session.dispatch(Action::Redo);
        assert_eq!(
            session.document().scene_set.as_ref().unwrap().scenes[1].data,
            vec![9, 9, 9, 9]
        );

        session.dispatch(Action::MarkSaved);
        assert!(!session.is_dirty());
    }

    #[test]
    fn rejects_out_of_range_scene() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        let outcome = session.dispatch(Action::SelectScene(2));
        assert_eq!(outcome.error, Some(CoreError::SceneIndex(2)));
    }

    #[test]
    fn edit_without_document_fails() {
        let mut session = EditorSession::new();
        let outcome = session.dispatch(Action::SetGlobalData(vec![0]));
        assert_eq!(outcome.error, Some(CoreError::NoGlobalData));
    }

    #[test]
    fn loading_global_keeps_scene_dirty_state() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        session.dispatch(Action::SetSceneData {
            index: 0,
            data: vec![9, 9, 9, 9],
        });
        assert!(session.is_dirty());
        session.dispatch(Action::LoadGlobal(GlobalData {
            model: FileModelKey::new("0820").unwrap(),
            data: vec![1, 2],
        }));
        assert!(session.is_dirty());
        assert_eq!(
            session.document().scene_set.as_ref().unwrap().scenes[0].data,
            vec![9, 9, 9, 9]
        );
    }

    #[test]
    fn rejects_wrong_scene_size() {
        let mut session = EditorSession::new();
        session.dispatch(Action::LoadSceneSet(scene_set()));
        let outcome = session.dispatch(Action::SetSceneData {
            index: 0,
            data: vec![1, 2],
        });
        assert_eq!(
            outcome.error,
            Some(CoreError::SceneSize {
                expected: 4,
                actual: 2
            })
        );
    }
}
