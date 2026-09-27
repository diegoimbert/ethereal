//! base-62 (`collab-social`, docs/COLLAB.md §12): chat journal and pinned notes in the
//! document. Validation caps, chat order assigned at apply (log order), pruning, the
//! History exemption for chat, and loading v3 files without the new tables.

use ether_model::*;

struct F {
    ids: IdGen,
    p: Project,
    h: History,
}

impl F {
    fn new() -> Self {
        let mut ids = IdGen::new(7);
        let p = Project::new(&mut ids, 1);
        Self {
            ids,
            p,
            h: History::new(0),
        }
    }

    fn author() -> Author {
        Author {
            name: "Ada".into(),
            site: Some(SiteId(42)),
            actor: None,
            color: Some(Color(0x5c_ffe8)),
        }
    }

    fn chat(&mut self, text: &str) -> ChatMessage {
        ChatMessage {
            id: self.ids.next(2),
            seq: 0,
            author: Self::author(),
            text: text.into(),
            sent_at: 1_700_000_000_000,
        }
    }

    fn note(&mut self, text: &str) -> PinnedNote {
        PinnedNote {
            id: self.ids.next(2),
            position: NotePosition {
                beats: Beats(8.0),
                track: None,
                y: 0.0,
            },
            text: text.into(),
            author: Self::author(),
            created_at: 1_700_000_000_000,
            resolved: false,
        }
    }

    fn commit(&mut self, ops: Vec<Op>) -> Result<Vec<Op>, ModelError> {
        let tx = Transaction {
            label: "t".into(),
            ops,
        };
        self.h.commit(&mut self.p, tx, None)
    }
}

fn insert(e: Entity) -> Op {
    Op::Insert { entity: e }
}

#[test]
fn chat_text_and_author_are_validated() {
    let mut f = F::new();
    for bad in ["", "   \n", &"x".repeat(CHAT_TEXT_MAX_CHARS + 1)] {
        let m = f.chat(bad);
        assert!(
            matches!(
                f.p.apply(&insert(Entity::ChatMessage(m))),
                Err(ModelError::InvalidValue(_))
            ),
            "{:?}",
            bad.len()
        );
    }
    // The cap counts characters, not bytes.
    let m = f.chat(&"é".repeat(CHAT_TEXT_MAX_CHARS));
    f.p.apply(&insert(Entity::ChatMessage(m))).unwrap();
    let mut m = f.chat("hi");
    m.author.name = "n".repeat(AUTHOR_NAME_MAX_CHARS + 1);
    assert!(f.p.apply(&insert(Entity::ChatMessage(m))).is_err());
    let mut m = f.chat("hi");
    m.author.color = Some(Color(0x1_000_000));
    assert!(f.p.apply(&insert(Entity::ChatMessage(m))).is_err());
}

#[test]
fn chat_seq_is_assigned_in_apply_order() {
    let mut f = F::new();
    let (a, b, c) = (f.chat("a"), f.chat("b"), f.chat("c"));
    // Applied c, a, b (the log order), whatever their ids or sent_at say.
    for m in [&c, &a, &b] {
        f.p.apply(&insert(Entity::ChatMessage(m.clone()))).unwrap();
    }
    let order: Vec<&str> = f.p.chat_ordered().iter().map(|m| m.text.as_str()).collect();
    assert_eq!(order, ["c", "a", "b"]);
    assert_eq!(f.p.chat[&b.id].seq, 3);
    // The inverse of a remove restores the message with its seq (not a new one).
    let inv =
        f.p.apply(&Op::Remove {
            key: EntityKey::ChatMessage(c.id),
        })
        .unwrap();
    f.p.apply(&inv).unwrap();
    assert_eq!(f.p.chat[&c.id].seq, 1);
    f.p.validate().unwrap();
}

#[test]
fn chat_seq_saturates_at_u64_max() {
    let mut f = F::new();
    // A forged (or loaded) message at the top of the range.
    let mut top = f.chat("forged");
    top.seq = u64::MAX;
    f.p.apply(&insert(Entity::ChatMessage(top.clone())))
        .unwrap();
    let next = f.chat("next");
    f.p.apply(&insert(Entity::ChatMessage(next.clone())))
        .unwrap();
    assert_eq!(f.p.chat[&next.id].seq, u64::MAX, "no panic, no wrap to 0");
    // Ties sort by id: the later ULID stays after the forged one.
    let order: Vec<ChatMessageId> = f.p.chat_ordered().iter().map(|m| m.id).collect();
    assert_eq!(order, [top.id, next.id]);
}

#[test]
fn chat_overflow_prunes_the_oldest() {
    let mut f = F::new();
    let ms: Vec<ChatMessage> = (0..5).map(|i| f.chat(&format!("m{i}"))).collect();
    for m in &ms {
        f.p.apply(&insert(Entity::ChatMessage(m.clone()))).unwrap();
    }
    // Room for one more at a cap of 3: the 3 oldest go.
    assert_eq!(f.p.chat_overflow(3), vec![ms[0].id, ms[1].id, ms[2].id]);
    assert!(f.p.chat_overflow(10).is_empty());
}

#[test]
fn chat_is_exempt_from_history() {
    let mut f = F::new();
    let marker = Marker {
        id: f.ids.next(2),
        position: Beats(4.0),
        name: "A".into(),
        color: None,
    };
    f.commit(vec![insert(Entity::Marker(marker.clone()))])
        .unwrap();
    let m = f.chat("hello");
    // A chat-only transaction applies (and is returned for patches/collab) but records
    // nothing: undo still undoes the marker.
    let applied = f
        .commit(vec![insert(Entity::ChatMessage(m.clone()))])
        .unwrap();
    assert_eq!(applied.len(), 1);
    assert!(f.p.chat.contains_key(&m.id));
    assert_eq!(f.h.state().undo_label.as_deref(), Some("t"));
    f.h.undo(&mut f.p).unwrap().unwrap();
    assert!(f.p.markers.is_empty());
    assert!(f.p.chat.contains_key(&m.id), "undo never removes a message");
    assert!(!f.h.state().can_undo);
    // A chat message between undo and redo keeps the redo stack.
    let m2 = f.chat("again");
    f.commit(vec![insert(Entity::ChatMessage(m2))]).unwrap();
    assert!(f.h.state().can_redo);
    f.h.redo(&mut f.p).unwrap().unwrap();
    assert!(f.p.markers.contains_key(&marker.id));
    // Mixed transaction: only the non-chat part becomes the step.
    let m3 = f.chat("mixed");
    let note = f.note("look here");
    f.commit(vec![
        insert(Entity::ChatMessage(m3.clone())),
        insert(Entity::PinnedNote(note.clone())),
    ])
    .unwrap();
    f.h.undo(&mut f.p).unwrap().unwrap();
    assert!(!f.p.pinned_notes.contains_key(&note.id));
    assert!(f.p.chat.contains_key(&m3.id));
    assert_eq!(f.p.chat.len(), 3);
}

#[test]
fn pinned_notes_validate_and_undo() {
    let mut f = F::new();
    let mut bad = f.note("x");
    bad.position.y = 1.5;
    assert!(f.commit(vec![insert(Entity::PinnedNote(bad))]).is_err());
    let mut bad = f.note("x");
    bad.position.beats = Beats(-1.0);
    assert!(f.commit(vec![insert(Entity::PinnedNote(bad))]).is_err());
    let bad = f.note(&"x".repeat(NOTE_TEXT_MAX_CHARS + 1));
    assert!(f.commit(vec![insert(Entity::PinnedNote(bad))]).is_err());

    // A weak track reference: a note may point at a track that does not exist.
    let mut n = f.note("chorus needs work");
    n.position.track = Some(TrackId::NIL);
    f.commit(vec![insert(Entity::PinnedNote(n.clone()))])
        .unwrap();
    f.commit(vec![Op::Update {
        update: EntityUpdate::PinnedNote {
            id: n.id,
            change: PinnedNoteChange::Resolved(true),
        },
    }])
    .unwrap();
    assert!(f.p.pinned_notes[&n.id].resolved);
    f.h.undo(&mut f.p).unwrap().unwrap();
    assert!(!f.p.pinned_notes[&n.id].resolved);
    f.h.undo(&mut f.p).unwrap().unwrap();
    assert!(f.p.pinned_notes.is_empty());
}

#[test]
fn v3_files_without_the_tables_load_and_new_ones_round_trip() {
    let mut f = F::new();
    let m = f.chat("saved with the project");
    let n = f.note("pinned");
    f.p.apply(&insert(Entity::ChatMessage(m.clone()))).unwrap();
    f.p.apply(&insert(Entity::PinnedNote(n))).unwrap();
    let json = file::save(&f.p, "0.1.0").unwrap();
    assert_eq!(file::load(&json).unwrap(), f.p);

    let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    let project = doc["project"].as_object_mut().unwrap();
    project.remove("chat");
    project.remove("pinned_notes");
    let loaded = file::load(&doc.to_string()).unwrap();
    assert!(loaded.chat.is_empty() && loaded.pinned_notes.is_empty());
    assert_eq!(doc["version"], file::CURRENT_VERSION);
}
