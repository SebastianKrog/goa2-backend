//! Worker-local mutable object identity. JSON conversion is boundary-only.
//! Live context/frame bindings use these handles; dense world slots remain separate.
use crate::{
    bitset::BitSet,
    codec::{ObjectBinding, ObjectKind, ObjectPathPart, StateEnvelope},
    data::{Datum, Symbol, Symbols},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(usize);
/// Dense arena handles make reachability membership a bitmap lookup. The
/// retained handles support copied-step protection; only membership is semantic.
#[derive(Clone, Debug)]
pub(crate) struct ProtectedObjects {
    membership: crate::bitset::BitSet,
    objects: Vec<ObjectId>,
}
impl ProtectedObjects {
    fn new(len: usize) -> Self {
        Self {
            membership: crate::bitset::BitSet::new(len),
            objects: Vec::new(),
        }
    }
    fn insert<const RETAIN_ORDER: bool>(&mut self, id: ObjectId) -> bool {
        if self.membership.insert(id.0) {
            if RETAIN_ORDER {
                self.objects.push(id);
            }
            true
        } else {
            false
        }
    }
    pub(crate) fn contains(&self, id: &ObjectId) -> bool {
        self.membership.contains(id.0)
    }
}
impl IntoIterator for ProtectedObjects {
    type Item = ObjectId;
    type IntoIter = std::vec::IntoIter<ObjectId>;
    fn into_iter(self) -> Self::IntoIter {
        self.objects.into_iter()
    }
}
#[derive(Clone, Debug, Hash)]
pub enum ObjectValue {
    Atom(Datum),
    Object(ObjectId),
    Request(u64),
    GeneratedStep(u64),
}
#[derive(Clone, Debug, Hash)]
enum Contents {
    List(Vec<ObjectValue>),
    Fields(Vec<(Symbol, ObjectValue)>),
}
#[derive(Clone, Debug, Hash)]
struct Object {
    kind: ObjectKind,
    tracked: bool,
    deepcopy_shared: bool,
    contents: Contents,
}
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Context,
    Alias(usize),
    Object(ObjectId),
}
#[derive(Clone, Debug)]
pub enum Input {
    Literal(Datum),
    Alias(usize),
    Existing(ObjectValue),
}
#[derive(Clone, Debug)]
pub enum Mutation {
    Append {
        target: Target,
        value: Input,
    },
    Set {
        target: Target,
        key: Symbol,
        value: Input,
    },
    Delete {
        target: Target,
        key: Symbol,
    },
    Pop {
        target: Target,
        index: i64,
    },
    Snapshot {
        key: Symbol,
    },
}
#[derive(Clone, Debug, Hash)]
struct PhysicalCopy {
    base: u32,
    equality: Option<crate::codec::InstanceEquality>,
    fields: Vec<(Symbol, Option<Datum>)>,
}
#[derive(Clone, Debug)]
enum ArenaUndo {
    Object(ObjectId, Arc<Object>),
    ListAppend(ObjectId),
    ListRemove(ObjectId, usize, ObjectValue),
    ListReplace(ObjectId, usize, ObjectValue),
    PhysicalFields(u32, Option<Vec<(Symbol, Option<Datum>)>>),
}

#[derive(Debug, Default)]
struct RecordedSlots {
    objects: BitSet,
    physical: BitSet,
}

impl RecordedSlots {
    fn union_with(&mut self, other: &Self) {
        self.objects.union_with(&other.objects);
        self.physical.union_with(&other.physical);
    }

    fn clear(&mut self) {
        self.objects.clear();
        self.physical.clear();
    }

    fn capacity(&self) -> usize {
        self.objects.capacity() + self.physical.capacity()
    }
}

#[derive(Debug, Default)]
struct ArenaJournal {
    entries: Vec<ArenaUndo>,
    recorded: RecordedSlots,
    spare_recorded: RecordedSlots,
    existing_objects: Option<usize>,
}

impl Clone for ArenaJournal {
    fn clone(&self) -> Self {
        // A worker fork starts from the current values with its own undo history.
        Self::default()
    }
}

pub(crate) struct ArenaCheckpoint {
    symbols: Arc<Symbols>,
    entry_count: usize,
    object_count: usize,
    copy_count: usize,
    parent_existing_objects: Option<usize>,
    parent_recorded: RecordedSlots,
}

struct ArenaRollback<'a> {
    arena: &'a mut ObjectArena,
    checkpoint: Option<ArenaCheckpoint>,
}

impl Drop for ArenaRollback<'_> {
    fn drop(&mut self) {
        self.arena.rollback_to(self.checkpoint.take().unwrap());
    }
}

#[derive(Clone)]
struct ObjectPage {
    objects: Vec<Arc<Object>>,
    fingerprint: OnceLock<[u64; 2]>,
}

impl ObjectPage {
    fn recompute_fingerprint(&self) -> [u64; 2] {
        use std::hash::{DefaultHasher, Hash, Hasher};

        [0u64, 1].map(|salt| {
            let mut state = DefaultHasher::new();
            (salt, &self.objects).hash(&mut state);
            state.finish()
        })
    }

    fn objects_mut(&mut self) -> &mut Vec<Arc<Object>> {
        self.fingerprint.take();
        &mut self.objects
    }
}

// Forks share pages of handles. The undo journal still records individual
// object edits, and cached hashes are invalidated only on edited pages.
#[derive(Clone, Default)]
struct ObjectPages {
    pages: Vec<Arc<ObjectPage>>,
    revisions: Vec<u64>,
    revision: u64,
    len: usize,
}

impl std::fmt::Debug for ObjectPages {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_list()
            .entries(self.pages.iter().map(|page| &page.objects))
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObjectPageStamp {
    page: usize,
    revision: u64,
}

impl ObjectPages {
    const PAGE_LEN: usize = 32;

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, index: usize) -> Option<&Arc<Object>> {
        self.pages
            .get(index / Self::PAGE_LEN)?
            .objects
            .get(index % Self::PAGE_LEN)
    }

    fn touch(&mut self, page: usize) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("arena revision exhausted");
        self.revisions[page] = self.revision;
    }

    fn push(&mut self, object: Arc<Object>) {
        if self.len % Self::PAGE_LEN == 0 {
            self.pages.push(Arc::new(ObjectPage {
                objects: Vec::with_capacity(Self::PAGE_LEN),
                fingerprint: OnceLock::new(),
            }));
            self.revisions.push(0);
        }
        self.touch(self.pages.len() - 1);
        Arc::make_mut(self.pages.last_mut().unwrap())
            .objects_mut()
            .push(object);
        self.len += 1;
    }

    fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        let pages = len.div_ceil(Self::PAGE_LEN);
        self.pages.truncate(pages);
        self.revisions.truncate(pages);
        let tail = len % Self::PAGE_LEN;
        if tail != 0 && self.pages.last().unwrap().objects.len() > tail {
            self.touch(pages - 1);
            Arc::make_mut(self.pages.last_mut().unwrap())
                .objects_mut()
                .truncate(tail);
        }
        self.len = len;
    }
}

impl std::ops::Index<usize> for ObjectPages {
    type Output = Arc<Object>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.pages[index / Self::PAGE_LEN].objects[index % Self::PAGE_LEN]
    }
}

impl std::ops::IndexMut<usize> for ObjectPages {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let page = index / Self::PAGE_LEN;
        self.touch(page);
        &mut Arc::make_mut(&mut self.pages[page]).objects_mut()[index % Self::PAGE_LEN]
    }
}

impl std::hash::Hash for ObjectPages {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.len.hash(state);
        for page in &self.pages {
            page.fingerprint
                .get_or_init(|| page.recompute_fingerprint())
                .hash(state);
        }
    }
}

#[cfg(test)]
impl FromIterator<Arc<Object>> for ObjectPages {
    fn from_iter<T: IntoIterator<Item = Arc<Object>>>(iter: T) -> Self {
        let mut pages = Self::default();
        for object in iter {
            pages.push(object);
        }
        pages
    }
}

#[derive(Debug, Default)]
struct ArenaLayout {
    aliases: Vec<ObjectId>,
    board_keys: Option<Vec<(crate::geometry::Hex, ObjectId)>>,
    private_context: Option<(usize, u64)>,
    fingerprint: OnceLock<[u64; 2]>,
}

impl ArenaLayout {
    fn fingerprint(&self) -> &[u64; 2] {
        use std::hash::{DefaultHasher, Hash, Hasher};

        self.fingerprint.get_or_init(|| {
            [0u64, 1].map(|salt| {
                let mut state = DefaultHasher::new();
                (salt, &self.aliases, &self.board_keys).hash(&mut state);
                state.finish()
            })
        })
    }
}

#[derive(Clone, Debug)]
pub struct ObjectArena {
    pub(crate) symbols: Arc<Symbols>,
    objects: ObjectPages,
    layout: Arc<ArenaLayout>,
    root: ObjectValue,
    context: ObjectId,
    base_instances: usize,
    base_equality: Arc<[Option<crate::codec::InstanceEquality>]>,
    physical_copies: Vec<PhysicalCopy>,
    physical_fields: HashMap<u32, Vec<(Symbol, Option<Datum>)>>,
    journal: ArenaJournal,
}
#[derive(Clone, Debug)]
pub(crate) enum ObjectSlot {
    Field(Symbol),
    Index(usize),
}

impl ObjectArena {
    /// Run a speculative arena operation and restore all arena mutations on exit.
    ///
    /// Scopes can nest. Arena handles allocated inside a scope expire when it
    /// exits. Session state outside this arena requires its own undo entries.
    pub fn with_rollback<T>(&mut self, operation: impl FnOnce(&mut Self) -> T) -> T {
        let checkpoint = self.begin_rollback();
        let scope = ArenaRollback {
            arena: self,
            checkpoint: Some(checkpoint),
        };
        operation(scope.arena)
    }

    pub(crate) fn begin_rollback(&mut self) -> ArenaCheckpoint {
        let object_count = self.objects.len();
        ArenaCheckpoint {
            symbols: self.symbols.clone(),
            entry_count: self.journal.entries.len(),
            object_count,
            copy_count: self.physical_copies.len(),
            parent_existing_objects: self.journal.existing_objects.replace(object_count),
            parent_recorded: std::mem::replace(
                &mut self.journal.recorded,
                std::mem::take(&mut self.journal.spare_recorded),
            ),
        }
    }

    pub(crate) fn rollback_to(&mut self, checkpoint: ArenaCheckpoint) {
        while self.journal.entries.len() > checkpoint.entry_count {
            match self.journal.entries.pop().unwrap() {
                ArenaUndo::Object(id, previous) => self.objects[id.0] = previous,
                ArenaUndo::ListAppend(id) => {
                    self.list_items_mut(id).pop();
                }
                ArenaUndo::ListRemove(id, index, previous) => {
                    self.list_items_mut(id).insert(index, previous);
                }
                ArenaUndo::ListReplace(id, index, previous) => {
                    self.list_items_mut(id)[index] = previous;
                }
                ArenaUndo::PhysicalFields(id, Some(previous)) => {
                    self.physical_fields.insert(id, previous);
                }
                ArenaUndo::PhysicalFields(id, None) => {
                    self.physical_fields.remove(&id);
                }
            }
        }
        self.objects.truncate(checkpoint.object_count);
        self.physical_copies.truncate(checkpoint.copy_count);
        self.symbols = checkpoint.symbols;
        self.journal.existing_objects = checkpoint.parent_existing_objects;
        self.restore_recorded_slots(checkpoint.parent_recorded, false);
    }

    pub(crate) fn commit_rollback(&mut self, checkpoint: ArenaCheckpoint) {
        self.journal.existing_objects = checkpoint.parent_existing_objects;
        let nested = checkpoint.parent_existing_objects.is_some();
        if !nested {
            self.journal.entries.truncate(checkpoint.entry_count);
        }
        self.restore_recorded_slots(checkpoint.parent_recorded, nested);
    }

    fn restore_recorded_slots(&mut self, parent: RecordedSlots, commit_nested: bool) {
        let mut finished = std::mem::replace(&mut self.journal.recorded, parent);
        if commit_nested {
            // Keep the child's undo entries so the enclosing rollback can
            // reverse the committed writes in order.
            self.journal.recorded.union_with(&finished);
        }
        // Reuse the allocation at the next sibling without retaining slots.
        if finished.capacity() > self.journal.spare_recorded.capacity() {
            finished.clear();
            self.journal.spare_recorded = finished;
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.journal.existing_objects.is_some()
    }

    fn object_mut(&mut self, id: ObjectId) -> &mut Object {
        if self
            .journal
            .existing_objects
            .is_some_and(|count| id.0 < count)
            && self.journal.recorded.objects.insert_growing(id.0)
        {
            self.journal
                .entries
                .push(ArenaUndo::Object(id, self.objects[id.0].clone()));
        }
        Arc::make_mut(&mut self.objects[id.0])
    }

    fn records_list_edit(&self, id: ObjectId) -> bool {
        self.journal
            .existing_objects
            .is_some_and(|count| id.0 < count)
            && !self.journal.recorded.objects.contains(id.0)
    }

    fn list_items_mut(&mut self, id: ObjectId) -> &mut Vec<ObjectValue> {
        let Contents::List(values) = &mut Arc::make_mut(&mut self.objects[id.0]).contents else {
            unreachable!("validated list binding");
        };
        values
    }

    fn physical_overrides_mut(&mut self, id: u32) -> &mut Vec<(Symbol, Option<Datum>)> {
        if self.journal.existing_objects.is_some()
            && self.journal.recorded.physical.insert_growing(id as usize)
        {
            self.journal.entries.push(ArenaUndo::PhysicalFields(
                id,
                self.physical_fields.get(&id).cloned(),
            ));
        }
        self.physical_fields.entry(id).or_default()
    }

    pub fn compile(source: &StateEnvelope) -> Result<Self, String> {
        source.validate()?;
        Self::compile_validated(source, Symbols::default())
    }

    pub(crate) fn compile_validated(
        source: &StateEnvelope,
        symbols: Symbols,
    ) -> Result<Self, String> {
        let mut arena = Self {
            symbols: Arc::new(symbols),
            objects: ObjectPages::default(),
            layout: Arc::default(),
            root: ObjectValue::Atom(Datum::Null),
            context: ObjectId(0),
            base_instances: source.instances.len(),
            base_equality: source
                .instances
                .iter()
                .map(|instance| instance.equality.clone())
                .collect(),
            physical_copies: Vec::new(),
            physical_fields: HashMap::new(),
            journal: ArenaJournal::default(),
        };
        let paths = source
            .object_bindings
            .iter()
            .enumerate()
            .flat_map(|(index, object)| {
                object
                    .paths
                    .iter()
                    .cloned()
                    .map(move |path| (path, (index, object.kind, object.deepcopy_shared)))
            })
            .collect::<HashMap<_, _>>();
        let mut shared = vec![None; source.object_bindings.len()];
        arena.root = arena.import(&source.state, &mut Vec::new(), &paths, &mut shared)?;
        let aliases = shared
            .into_iter()
            .map(|id| id.ok_or("unresolved shared object".to_string()))
            .collect::<Result<_, _>>()?;
        let ObjectValue::Object(root) = arena.root else {
            return Err("object arena state root must be an object".into());
        };
        let Contents::Fields(fields) = &arena.objects[root.0].contents else {
            return Err("object arena state root must have fields".into());
        };
        arena.context = fields
            .iter()
            .find(|(key, _)| arena.symbols.name(*key) == "execution_context")
            .and_then(|(_, value)| {
                if let ObjectValue::Object(id) = value {
                    Some(*id)
                } else {
                    None
                }
            })
            .ok_or("object arena execution context missing")?;
        if arena.objects[arena.context.0].kind != ObjectKind::Dict {
            return Err("object arena execution context must be a dictionary".into());
        }
        let board_keys = if let Some(keys) = &source.board_keys {
            let coordinates =
                ["q", "r", "s"].map(|name| Arc::make_mut(&mut arena.symbols).intern(name));
            let mut bound = Vec::new();
            for key in keys {
                let id = if let Some(path) = &key.path {
                    let ObjectValue::Object(id) = arena.value_at_path(path)? else {
                        return Err("board-key alias model missing".into());
                    };
                    if !arena.is_shared_frozen_model(id) {
                        return Err("board-key alias must be an immutable Hex".into());
                    }
                    id
                } else {
                    let (q, r, s) = key.hex.coordinates();
                    let ObjectValue::Object(id) = arena.shared_frozen_model(
                        coordinates
                            .into_iter()
                            .zip([q, r, s])
                            .map(|(key, value)| {
                                (key, ObjectValue::Atom(Datum::Int(i64::from(value))))
                            })
                            .collect(),
                    )?
                    else {
                        unreachable!()
                    };
                    id
                };
                bound.push((key.hex, id));
            }
            Some(bound)
        } else {
            None
        };
        let private_context = arena
            .context_edges_are_private(0, None)
            .then_some((arena.objects.len(), arena.objects.revision));
        arena.layout = Arc::new(ArenaLayout {
            aliases,
            board_keys,
            private_context,
            fingerprint: OnceLock::new(),
        });
        Ok(arena)
    }
    pub(crate) fn board_key(&self, index: usize) -> Option<ObjectId> {
        self.layout
            .board_keys
            .as_ref()?
            .get(index)
            .map(|(_, id)| *id)
    }
    fn import(
        &mut self,
        value: &Value,
        path: &mut Vec<ObjectPathPart>,
        paths: &HashMap<Vec<ObjectPathPart>, (usize, ObjectKind, bool)>,
        shared: &mut [Option<ObjectId>],
    ) -> Result<ObjectValue, String> {
        let binding = paths.get(path).copied();
        if let Some((index, _, _)) = binding
            && let Some(id) = shared[index]
        {
            return Ok(ObjectValue::Object(id));
        }
        if value.is_array() || (value.is_object() && value.get("$goa2_instance").is_none()) {
            let kind = binding.map_or(
                if value.is_array() {
                    ObjectKind::List
                } else {
                    ObjectKind::Dict
                },
                |(_, kind, _)| kind,
            );
            let id = ObjectId(self.objects.len());
            self.objects.push(Arc::new(Object {
                kind,
                tracked: binding.is_some(),
                deepcopy_shared: binding.is_some_and(|(_, _, shared)| shared),
                contents: Contents::List(Vec::new()),
            }));
            if let Some((index, _, _)) = binding {
                shared[index] = Some(id);
            }
            let contents = match value {
                Value::Array(items) => {
                    let mut result = Vec::new();
                    for (index, item) in items.iter().enumerate() {
                        path.push(ObjectPathPart::Index(index));
                        result.push(self.import(item, path, paths, shared)?);
                        path.pop();
                    }
                    Contents::List(result)
                }
                Value::Object(fields) => {
                    let mut result = Vec::new();
                    for (key, item) in fields {
                        let symbol = Arc::make_mut(&mut self.symbols).intern(key);
                        path.push(ObjectPathPart::Key(key.clone()));
                        result.push((symbol, self.import(item, path, paths, shared)?));
                        path.pop();
                    }
                    Contents::Fields(result)
                }
                _ => unreachable!(),
            };
            Arc::make_mut(&mut self.objects[id.0]).contents = contents;
            Ok(ObjectValue::Object(id))
        } else {
            // Setup records an unsigned seed at the root. Rule arithmetic
            // and ordinary input values still use signed integers.
            if let [ObjectPathPart::Key(key)] = path.as_slice()
                && key == "rng_seed"
                && let Some(seed) = value.as_u64().filter(|&seed| seed > i64::MAX as u64)
            {
                return Ok(ObjectValue::Atom(Datum::Unsigned(seed)));
            }
            Ok(ObjectValue::Atom(Datum::from_json(
                value,
                Arc::make_mut(&mut self.symbols),
            )?))
        }
    }
    fn target(&self, target: Target) -> Result<ObjectId, String> {
        match target {
            Target::Context => Ok(self.context),
            Target::Object(id) => self
                .objects
                .get(id.0)
                .map(|_| id)
                .ok_or("object handle outside arena".into()),
            Target::Alias(index) => self
                .layout
                .aliases
                .get(index)
                .copied()
                .ok_or("alias index outside table".into()),
        }
    }
    fn literal(&mut self, value: &Datum) -> ObjectValue {
        self.literal_with(value, None)
    }
    fn literal_with(
        &mut self,
        value: &Datum,
        replacement: Option<(Symbol, &Datum)>,
    ) -> ObjectValue {
        if let (Datum::Text(value), Some((from, to))) = (value, replacement)
            && *value == from
        {
            return ObjectValue::Atom(to.clone());
        }
        let (kind, contents) = match value {
            Datum::List(items) => (
                ObjectKind::List,
                Contents::List(
                    items
                        .iter()
                        .map(|v| self.literal_with(v, replacement))
                        .collect(),
                ),
            ),
            Datum::Object(fields) => (
                ObjectKind::Dict,
                Contents::Fields(
                    fields
                        .iter()
                        .map(|(k, v)| (*k, self.literal_with(v, replacement)))
                        .collect(),
                ),
            ),
            _ => return ObjectValue::Atom(value.clone()),
        };
        self.allocate_contents(kind, contents)
    }

    fn allocate_contents(&mut self, kind: ObjectKind, contents: Contents) -> ObjectValue {
        let id = ObjectId(self.objects.len());
        self.objects.push(Arc::new(Object {
            kind,
            tracked: true,
            deepcopy_shared: false,
            contents,
        }));
        ObjectValue::Object(id)
    }
    fn input(&mut self, value: &Input) -> Result<ObjectValue, String> {
        match value {
            Input::Literal(v) => Ok(self.literal(v)),
            Input::Existing(value) => Ok(value.clone()),
            Input::Alias(index) => Ok(ObjectValue::Object(self.target(Target::Alias(*index))?)),
        }
    }
    fn reaches(&self, value: &ObjectValue, target: ObjectId) -> bool {
        let ObjectValue::Object(root) = value else {
            return false;
        };
        let mut pending = Vec::new();
        let mut visited = None;
        let mut next = Some(*root);
        while let Some(id) = next.take() {
            if id == target {
                return true;
            }
            let mut enqueue = |value: &ObjectValue| {
                // Keep the next edge in a local slot. A linear path needs
                // no allocation for deferred siblings.
                if let ObjectValue::Object(child) = value
                    && let Some(sibling) = next.replace(*child)
                {
                    pending.push(sibling);
                }
            };
            match &self.objects[id.0].contents {
                Contents::List(items) => items.iter().for_each(&mut enqueue),
                Contents::Fields(fields) => {
                    fields.iter().for_each(|(_, value)| enqueue(value));
                }
            }
            if next.is_none() && pending.is_empty() {
                return false;
            }
            let seen =
                visited.get_or_insert_with(HashSet::<_, foldhash::fast::RandomState>::default);
            seen.insert(id);
            if next.is_none_or(|child| seen.contains(&child)) {
                next = std::iter::from_fn(|| pending.pop()).find(|child| !seen.contains(child));
            }
        }
        false
    }

    fn check_existing_input(&self, value: &ObjectValue, target: ObjectId) -> Result<(), String> {
        if self.reaches(value, target) {
            return Err("cyclic aggregate mutation".into());
        }
        Ok(())
    }

    fn check_input(&self, value: &Input, target: ObjectId) -> Result<(), String> {
        match value {
            Input::Alias(index) => self.check_existing_input(
                &ObjectValue::Object(self.target(Target::Alias(*index))?),
                target,
            ),
            Input::Existing(value) => self.check_existing_input(value, target),
            Input::Literal(_) => Ok(()),
        }
    }
    pub fn value_at_path(&self, path: &[ObjectPathPart]) -> Result<ObjectValue, String> {
        let mut value = self.root.clone();
        for part in path {
            let ObjectValue::Object(id) = value else {
                return Err("path crosses an atomic value".into());
            };
            value = match (part, &self.objects[id.0].contents) {
                (ObjectPathPart::Key(key), Contents::Fields(fields)) => fields
                    .iter()
                    .find(|(symbol, _)| self.symbols.name(*symbol) == key)
                    .map(|(_, value)| value.clone()),
                (ObjectPathPart::Index(index), Contents::List(items)) => items.get(*index).cloned(),
                _ => None,
            }
            .ok_or("object path missing")?;
        }
        Ok(value)
    }
    pub(crate) fn fresh_context_object_is_private(&self, object: ObjectId) -> bool {
        self.layout
            .private_context
            .is_some_and(|(initial_len, revision)| {
                object.0 >= initial_len && self.context_edges_are_private(revision, Some(object))
            })
    }

    // A new handle cannot occur in an untouched import page. The protected
    // walk excludes the root's context field. If the root and context have
    // no other incoming edges, a handle mentioned only by context is private.
    // Inspect all changed edges, including those excluded by the full walk,
    // so uncertain cases simply use ordinary protection.
    fn context_edges_are_private(&self, since: u64, target: Option<ObjectId>) -> bool {
        let ObjectValue::Object(root) = self.root else {
            return false;
        };
        let context_key = self.symbols.lookup("execution_context");
        for (index, (page, &revision)) in self
            .objects
            .pages
            .iter()
            .zip(&self.objects.revisions)
            .enumerate()
        {
            if revision <= since {
                continue;
            }
            for (offset, object) in page.objects.iter().enumerate() {
                let owner = ObjectId(index * ObjectPages::PAGE_LEN + offset);
                let forbidden = |value: &ObjectValue| {
                    matches!(value, ObjectValue::Object(id) if *id == root
                        || *id == self.context || (Some(*id) == target && owner != self.context))
                };
                let unsafe_edge = match &object.contents {
                    Contents::List(items) => items.iter().any(forbidden),
                    Contents::Fields(fields) => fields.iter().any(|(key, value)| {
                        forbidden(value)
                            && !(owner == root
                                && Some(*key) == context_key
                                && matches!(value, ObjectValue::Object(id) if *id == self.context))
                    }),
                };
                if unsafe_edge {
                    return false;
                }
            }
        }
        true
    }

    /// Objects reachable from immutable world/step parameters cannot yet be
    /// mutated by the scalar executor. Set.value and pending input are live
    /// arena payloads; all other step operands still compile as definitions.
    pub(crate) fn readonly_objects(&self) -> ProtectedObjects {
        let ObjectValue::Object(root) = self.root else {
            return ProtectedObjects::new(self.objects.len());
        };
        let Contents::Fields(fields) = &self.objects[root.0].contents else {
            return ProtectedObjects::new(self.objects.len());
        };
        let context = self.symbols.lookup("execution_context");
        let roots = fields
            .iter()
            .filter(|(key, _)| Some(*key) != context)
            .filter_map(|(_, value)| {
                if let ObjectValue::Object(id) = value {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        self.readonly_reachable::<false>(roots)
    }
    pub(crate) fn readonly_step_objects(&self, step: ObjectId) -> ProtectedObjects {
        self.readonly_reachable::<true>(vec![step])
    }
    fn readonly_reachable<const RETAIN_ORDER: bool>(
        &self,
        mut queue: Vec<ObjectId>,
    ) -> ProtectedObjects {
        let mut protected = ProtectedObjects::new(self.objects.len());
        // Resolve exclusions once per walk; scalar fields cannot add graph edges.
        let live_fields = [
            "value",
            "pending_input",
            "expire_effect_ids",
            "tied_hero_ids",
            "processed_effect_ids",
        ]
        .map(|name| self.symbols.lookup(name));
        while let Some(id) = queue.pop() {
            if !protected.insert::<RETAIN_ORDER>(id) {
                continue;
            }
            let object = &self.objects[id.0];
            match &object.contents {
                Contents::List(items) => {
                    for value in items {
                        if let ObjectValue::Object(id) = value {
                            queue.push(*id);
                        }
                    }
                }
                Contents::Fields(fields) => {
                    for (key, value) in fields {
                        if let ObjectValue::Object(id) = value
                            && (object.kind != ObjectKind::Step
                                || !live_fields.contains(&Some(*key)))
                        {
                            queue.push(*id);
                        }
                    }
                }
            }
        }
        protected
    }
    pub(crate) fn page_stamp(&self, object: ObjectId) -> Option<ObjectPageStamp> {
        let page = object.0 / ObjectPages::PAGE_LEN;
        Some(ObjectPageStamp {
            page,
            revision: *self.objects.revisions.get(page)?,
        })
    }

    pub(crate) fn matches_page_stamp(&self, stamp: ObjectPageStamp) -> bool {
        self.objects.revisions.get(stamp.page) == Some(&stamp.revision)
    }

    pub fn sequence(&self, object: ObjectId) -> Option<&[ObjectValue]> {
        match &self.objects.get(object.0)?.contents {
            Contents::List(values) => Some(values),
            _ => None,
        }
    }
    /// Python deepcopy uses one memo per template root, not per iteration.
    /// New physical identities share immutable definitions and snapshot current
    /// mutable overrides; later writes to the original cannot change a copy.
    pub(crate) fn deep_copy(
        &mut self,
        value: &ObjectValue,
    ) -> Result<(ObjectValue, HashMap<ObjectId, ObjectId>), String> {
        let mut objects = HashMap::new();
        let mut instances = HashMap::new();
        let value = self.copy_value(value, &mut objects, &mut instances)?;
        Ok((value, objects))
    }
    fn copy_value(
        &mut self,
        value: &ObjectValue,
        objects: &mut HashMap<ObjectId, ObjectId>,
        instances: &mut HashMap<u32, u32>,
    ) -> Result<ObjectValue, String> {
        Ok(match value {
            ObjectValue::Atom(Datum::Instance(id)) => {
                if let Some(copy) = instances.get(id) {
                    return Ok(ObjectValue::Atom(Datum::Instance(*copy)));
                }
                let next = self
                    .base_instances
                    .checked_add(self.physical_copies.len())
                    .ok_or("physical instance counter exhausted")?;
                if *id as usize >= next {
                    return Err("physical copy source outside table".into());
                }
                let next =
                    u32::try_from(next).map_err(|_| "physical instance counter exhausted")?;
                let mut copy = if (*id as usize) < self.base_instances {
                    PhysicalCopy {
                        base: *id,
                        equality: self.base_equality[*id as usize].clone(),
                        fields: Vec::new(),
                    }
                } else {
                    self.physical_copies[*id as usize - self.base_instances].clone()
                };
                if let Some(fields) = self.physical_fields.get(id) {
                    for (key, value) in fields {
                        Self::set_physical_override(&mut copy.fields, *key, value.clone());
                    }
                }
                self.physical_copies.push(copy);
                instances.insert(*id, next);
                ObjectValue::Atom(Datum::Instance(next))
            }
            ObjectValue::Object(id) => {
                if let Some(copy) = objects.get(id) {
                    return Ok(ObjectValue::Object(*copy));
                }
                if self.objects[id.0].deepcopy_shared {
                    objects.insert(*id, *id);
                    return Ok(ObjectValue::Object(*id));
                }
                let original = self.objects[id.0].clone();
                let copy = ObjectId(self.objects.len());
                self.objects.push(Arc::new(Object {
                    kind: original.kind,
                    tracked: original.tracked,
                    deepcopy_shared: original.deepcopy_shared,
                    contents: Contents::List(Vec::new()),
                }));
                objects.insert(*id, copy);
                let contents = match &original.contents {
                    Contents::List(values) => Contents::List(
                        values
                            .iter()
                            .map(|value| self.copy_value(value, objects, instances))
                            .collect::<Result<_, _>>()?,
                    ),
                    Contents::Fields(fields) => Contents::Fields(
                        fields
                            .iter()
                            .map(|(key, value)| {
                                Ok((*key, self.copy_value(value, objects, instances)?))
                            })
                            .collect::<Result<_, String>>()?,
                    ),
                };
                Arc::make_mut(&mut self.objects[copy.0]).contents = contents;
                ObjectValue::Object(copy)
            }
            value => value.clone(),
        })
    }
    fn set_physical_override(
        fields: &mut Vec<(Symbol, Option<Datum>)>,
        key: Symbol,
        value: Option<Datum>,
    ) {
        if let Some((_, old)) = fields.iter_mut().find(|(field, _)| *field == key) {
            *old = value;
        } else {
            fields.push((key, value));
        }
    }
    pub(crate) fn physical_base(&self, id: u32) -> Result<u32, String> {
        if (id as usize) < self.base_instances {
            Ok(id)
        } else {
            self.physical_copies
                .get(id as usize - self.base_instances)
                .map(|copy| copy.base)
                .ok_or("physical instance missing".into())
        }
    }
    pub(crate) fn physical_override(&self, id: u32, key: Symbol) -> Option<Option<&Datum>> {
        self.physical_fields
            .get(&id)
            .and_then(|fields| fields.iter().find(|(field, _)| *field == key))
            .or_else(|| {
                (id as usize >= self.base_instances)
                    .then(|| self.physical_copies.get(id as usize - self.base_instances))
                    .flatten()
                    .and_then(|copy| copy.fields.iter().find(|(field, _)| *field == key))
            })
            .map(|(_, value)| value.as_ref())
    }
    pub(crate) fn set_physical_field(
        &mut self,
        instance: u32,
        key: Symbol,
        value: Datum,
    ) -> Result<(), String> {
        if instance as usize >= self.base_instances + self.physical_copies.len() {
            return Err("physical field target outside table".into());
        }
        if matches!(
            value,
            Datum::List(_) | Datum::Object(_) | Datum::Instance(_)
        ) {
            return Err("physical aggregate field bindings are not implemented".into());
        }
        Self::set_physical_override(self.physical_overrides_mut(instance), key, Some(value));
        Ok(())
    }
    pub(crate) fn remove_physical_field(
        &mut self,
        instance: u32,
        key: Symbol,
    ) -> Result<(), String> {
        if instance as usize >= self.base_instances + self.physical_copies.len() {
            return Err("physical field target outside table".into());
        }
        Self::set_physical_override(self.physical_overrides_mut(instance), key, None);
        Ok(())
    }
    pub(crate) fn root_field(&self, key: Symbol) -> Option<ObjectValue> {
        let ObjectValue::Object(root) = self.root else {
            return None;
        };
        self.field(root, key)
    }
    /// A source tuple has no mutable container identity; its model children do.
    pub(crate) fn tuple(&mut self, values: Vec<ObjectValue>) -> ObjectValue {
        let ObjectValue::Object(id) = self.list(values) else {
            unreachable!()
        };
        Arc::make_mut(&mut self.objects[id.0]).tracked = false;
        ObjectValue::Object(id)
    }
    pub(crate) fn frozen_model(
        &mut self,
        fields: Vec<(Symbol, ObjectValue)>,
    ) -> Result<ObjectValue, String> {
        let value = self.dictionary(fields)?;
        let ObjectValue::Object(id) = value else {
            unreachable!()
        };
        Arc::make_mut(&mut self.objects[id.0]).kind = ObjectKind::FrozenModel;
        Ok(ObjectValue::Object(id))
    }
    /// Explicitly immutable source models (Hex) return themselves from deepcopy.
    /// Other frozen models still use ordinary per-template deepcopy identities.
    pub(crate) fn shared_frozen_model(
        &mut self,
        fields: Vec<(Symbol, ObjectValue)>,
    ) -> Result<ObjectValue, String> {
        let value = self.frozen_model(fields)?;
        let ObjectValue::Object(id) = value else {
            unreachable!()
        };
        Arc::make_mut(&mut self.objects[id.0]).deepcopy_shared = true;
        Ok(ObjectValue::Object(id))
    }
    pub(crate) fn list(&mut self, values: Vec<ObjectValue>) -> ObjectValue {
        let id = ObjectId(self.objects.len());
        self.objects.push(Arc::new(Object {
            kind: ObjectKind::List,
            tracked: true,
            deepcopy_shared: false,
            contents: Contents::List(values),
        }));
        ObjectValue::Object(id)
    }
    /// A cache payload only: authoritative identity remains in the arena graph.
    pub(crate) fn typed_value(
        &self,
        value: &ObjectValue,
        symbols: &mut Symbols,
    ) -> Result<Datum, String> {
        fn read(
            arena: &ObjectArena,
            value: &ObjectValue,
            ancestors: &mut HashSet<ObjectId>,
            symbols: &mut Symbols,
        ) -> Result<Datum, String> {
            let id = match value {
                ObjectValue::Object(id) => id,
                ObjectValue::Atom(value) => return Ok(value.clone()),
                ObjectValue::Request(id) => {
                    return Ok(Datum::Text(symbols.intern(&format!("native-request-{id}"))));
                }
                ObjectValue::GeneratedStep(id) => {
                    return Ok(Datum::Text(symbols.intern(&format!("native-step-{id}"))));
                }
            };
            if ancestors.len() >= 256 || !ancestors.insert(*id) {
                return Err("typed effect payload contains an unsupported cycle/depth".into());
            }
            let result = if let Some(values) = arena.sequence(*id) {
                Datum::List(
                    values
                        .iter()
                        .map(|value| read(arena, value, ancestors, symbols))
                        .collect::<Result<_, _>>()?,
                )
            } else if let Some(fields) = arena.fields(*id) {
                Datum::Object(
                    fields
                        .iter()
                        .map(|(key, value)| Ok((*key, read(arena, value, ancestors, symbols)?)))
                        .collect::<Result<_, String>>()?,
                )
            } else {
                return Err("typed effect object has no contents".into());
            };
            ancestors.remove(id);
            Ok(result)
        }
        read(self, value, &mut HashSet::new(), symbols)
    }
    pub(crate) fn fields(&self, object: ObjectId) -> Option<&[(Symbol, ObjectValue)]> {
        match &self.objects.get(object.0)?.contents {
            Contents::Fields(fields) => Some(fields),
            _ => None,
        }
    }
    pub(crate) fn set_index(
        &mut self,
        object: ObjectId,
        index: usize,
        value: ObjectValue,
    ) -> Result<(), String> {
        if !self.is_mutable_list(object) {
            return Err("indexed assignment requires a mutable list".into());
        }
        if self.reaches(&value, object) {
            return Err("aggregate cycle".into());
        }
        let Contents::List(items) = &mut Arc::make_mut(&mut self.objects[object.0]).contents else {
            return Err("indexed assignment requires a list".into());
        };
        let target = items
            .get_mut(index)
            .ok_or("list assignment index out of range")?;
        let previous = std::mem::replace(target, value);
        if self.records_list_edit(object) {
            self.journal
                .entries
                .push(ArenaUndo::ListReplace(object, index, previous));
        }
        Ok(())
    }
    pub fn field(&self, object: ObjectId, key: Symbol) -> Option<ObjectValue> {
        self.field_ref(object, key).cloned()
    }

    pub(crate) fn field_ref(&self, object: ObjectId, key: Symbol) -> Option<&ObjectValue> {
        self.fields(object)?
            .iter()
            .find(|(field, _)| *field == key)
            .map(|(_, value)| value)
    }
    pub(crate) fn set_root(&mut self, key: Symbol, value: ObjectValue) -> Result<(), String> {
        let ObjectValue::Object(root) = self.root else {
            return Err("state root missing".into());
        };
        self.set_field(root, key, value)
    }
    pub(crate) fn dictionary(
        &mut self,
        fields: Vec<(Symbol, ObjectValue)>,
    ) -> Result<ObjectValue, String> {
        let value = self.allocate(&Datum::Object(Vec::new()));
        let ObjectValue::Object(id) = value else {
            unreachable!()
        };
        for (key, value) in fields {
            self.set_field(id, key, value)?;
        }
        Ok(ObjectValue::Object(id))
    }
    pub(crate) fn context_id(&self) -> ObjectId {
        self.context
    }
    pub fn context_value(&self, key: Symbol) -> Option<ObjectValue> {
        self.field(self.context, key)
    }
    /// Unclassified arrays can be source tuples. Only captured Python lists
    /// and freshly allocated native lists support in-place list operations.
    pub fn is_mutable_list(&self, object: ObjectId) -> bool {
        self.objects[object.0].tracked && self.objects[object.0].kind == ObjectKind::List
    }
    pub(crate) fn is_shared_frozen_model(&self, object: ObjectId) -> bool {
        self.objects[object.0].kind == ObjectKind::FrozenModel
            && self.objects[object.0].deepcopy_shared
    }
    pub fn kind(&self, object: ObjectId) -> ObjectKind {
        self.objects[object.0].kind
    }
    pub fn truthy(&self, value: &ObjectValue) -> bool {
        match value {
            ObjectValue::Atom(value) => value.truthy(&self.symbols),
            ObjectValue::Request(_) | ObjectValue::GeneratedStep(_) => true,
            ObjectValue::Object(id) => {
                match (&self.objects[id.0].kind, &self.objects[id.0].contents) {
                    (ObjectKind::List, Contents::List(items)) => !items.is_empty(),
                    (ObjectKind::Dict, Contents::Fields(fields)) => !fields.is_empty(),
                    _ => true,
                }
            }
        }
    }
    /// Instantiate a cold-compiled step recipe without serializing game state.
    pub(crate) fn step(&mut self, value: &Datum) -> Result<ObjectId, String> {
        let ObjectValue::Object(id) = self.literal(value) else {
            return Err("step recipe must have fields".into());
        };
        if !matches!(self.objects[id.0].contents, Contents::Fields(_)) {
            return Err("step recipe must have fields".into());
        }
        Arc::make_mut(&mut self.objects[id.0]).kind = ObjectKind::Step;
        Ok(id)
    }
    pub(crate) fn step_substituting_value(
        &mut self,
        value: &Datum,
        from: Symbol,
        to: &Datum,
    ) -> Result<ObjectId, String> {
        let ObjectValue::Object(id) = self.literal_with(value, Some((from, to))) else {
            return Err("step recipe must have fields".into());
        };
        if !matches!(self.objects[id.0].contents, Contents::Fields(_)) {
            return Err("step recipe must have fields".into());
        }
        Arc::make_mut(&mut self.objects[id.0]).kind = ObjectKind::Step;
        Ok(id)
    }
    pub(crate) fn recipe_object(
        &self,
        root: ObjectId,
        path: &[ObjectSlot],
    ) -> Result<ObjectId, String> {
        let mut object = root;
        for slot in path {
            let value = match slot {
                ObjectSlot::Field(key) => self.field(object, *key),
                ObjectSlot::Index(index) => self
                    .sequence(object)
                    .and_then(|values| values.get(*index))
                    .cloned(),
            }
            .ok_or("recipe path missing")?;
            let ObjectValue::Object(next) = value else {
                return Err("recipe path must name an object".into());
            };
            object = next;
        }
        Ok(object)
    }
    pub(crate) fn alias_recipe_object(
        &mut self,
        root: ObjectId,
        path: &[ObjectSlot],
        target: ObjectId,
    ) -> Result<(), String> {
        let (last, parent) = path
            .split_last()
            .ok_or("recipe root cannot be replaced by alias")?;
        let parent = self.recipe_object(root, parent)?;
        match last {
            ObjectSlot::Field(key) => self.set_field(parent, *key, ObjectValue::Object(target)),
            ObjectSlot::Index(index) => self.set_index(parent, *index, ObjectValue::Object(target)),
        }
    }
    pub(crate) fn classify_recipe_object(
        &mut self,
        root: ObjectId,
        path: &[ObjectSlot],
        kind: ObjectKind,
    ) -> Result<ObjectId, String> {
        let mut value = ObjectValue::Object(root);
        for part in path {
            let ObjectValue::Object(id) = value else {
                return Err("recipe path is not an object".into());
            };
            value = match part {
                ObjectSlot::Field(key) => self.field(id, *key),
                ObjectSlot::Index(index) => self
                    .sequence(id)
                    .and_then(|values| values.get(*index))
                    .cloned(),
            }
            .ok_or("recipe object path missing")?;
        }
        let ObjectValue::Object(id) = value else {
            return Err("recipe model missing".into());
        };
        self.object_mut(id).kind = kind;
        Ok(id)
    }
    /// Pydantic model_copy(deep=False): a new step retains child identities.
    pub(crate) fn shallow_step(&mut self, original: ObjectId) -> Result<ObjectId, String> {
        if self.kind(original) != ObjectKind::Step {
            return Err("shallow copy requires a step".into());
        }
        let copy = self.objects[original.0].clone();
        let id = ObjectId(self.objects.len());
        self.objects.push(copy);
        Ok(id)
    }
    pub fn allocate(&mut self, value: &Datum) -> ObjectValue {
        self.literal(value)
    }
    pub(crate) fn allocate_input(&mut self, key: Symbol, selection: &Datum) -> ObjectValue {
        let value = self.literal(selection);
        self.allocate_contents(ObjectKind::Dict, Contents::Fields(vec![(key, value)]))
    }

    pub fn set_field(
        &mut self,
        object: ObjectId,
        key: Symbol,
        value: ObjectValue,
    ) -> Result<(), String> {
        self.apply(&Mutation::Set {
            target: Target::Object(object),
            key,
            value: Input::Existing(value),
        })
    }
    pub fn set_context(&mut self, key: Symbol, value: ObjectValue) -> Result<(), String> {
        self.set_field(self.context, key, value)
    }
    pub fn remove_context(&mut self, key: Symbol) -> Result<(), String> {
        if self.context_value(key).is_some() {
            self.apply(&Mutation::Delete {
                target: Target::Context,
                key,
            })?;
        }
        Ok(())
    }
    pub(crate) fn check_append(&self, object: ObjectId, value: &ObjectValue) -> Result<(), String> {
        if !self.is_mutable_list(object) {
            return Err("append requires a mutable list".into());
        }
        self.check_existing_input(value, object)
    }
    pub fn append(&mut self, object: ObjectId, value: ObjectValue) -> Result<(), String> {
        self.apply(&Mutation::Append {
            target: Target::Object(object),
            value: Input::Existing(value),
        })
    }
    pub fn list_pop(&mut self, object: ObjectId) -> Result<Option<ObjectValue>, String> {
        let index = self
            .sequence(object)
            .ok_or("stack binding is not a list")?
            .len();
        let item = self.list_items_mut(object).pop();
        if let Some(value) = &item
            && self.records_list_edit(object)
        {
            self.journal
                .entries
                .push(ArenaUndo::ListRemove(object, index - 1, value.clone()));
        }
        Ok(item)
    }
    pub(crate) fn clear_dictionary(&mut self, object: ObjectId) -> Result<(), String> {
        if self.objects[object.0].kind != ObjectKind::Dict {
            return Err("clear requires a dictionary".into());
        }
        let Contents::Fields(fields) = &mut self.object_mut(object).contents else {
            return Err("dictionary fields missing".into());
        };
        fields.clear();
        Ok(())
    }
    pub fn clear_list(&mut self, object: ObjectId) -> Result<(), String> {
        let Contents::List(items) = &mut self.object_mut(object).contents else {
            return Err("stack binding is not a list".into());
        };
        items.clear();
        Ok(())
    }
    pub fn apply(&mut self, mutation: &Mutation) -> Result<(), String> {
        match mutation {
            Mutation::Append { target, value } => {
                let id = self.target(*target)?;
                if !matches!(self.objects[id.0].contents, Contents::List(_)) {
                    return Err("append target must be a list".into());
                }
                self.check_input(value, id)?;
                let value = self.input(value)?;
                self.list_items_mut(id).push(value);
                if self.records_list_edit(id) {
                    self.journal.entries.push(ArenaUndo::ListAppend(id));
                }
            }
            Mutation::Set { target, key, value } => {
                let id = self.target(*target)?;
                if self.objects[id.0].kind == ObjectKind::FrozenModel {
                    return Err("frozen model cannot be mutated".into());
                }
                let Contents::Fields(fields) = &self.objects[id.0].contents else {
                    return Err("set target must have fields".into());
                };
                let index = fields.iter().position(|(field, _)| field == key);
                if index.is_none()
                    && matches!(
                        self.objects[id.0].kind,
                        ObjectKind::Model | ObjectKind::Step
                    )
                {
                    return Err("model field does not exist".into());
                }
                self.check_input(value, id)?;
                let value = self.input(value)?;
                let Contents::Fields(fields) = &mut self.object_mut(id).contents else {
                    unreachable!()
                };
                if let Some(index) = index {
                    fields[index].1 = value;
                } else {
                    fields.push((*key, value));
                }
            }
            Mutation::Delete { target, key } => {
                let id = self.target(*target)?;
                if self.objects[id.0].kind != ObjectKind::Dict {
                    return Err("delete target must be a dictionary".into());
                }
                let Contents::Fields(fields) = &mut self.object_mut(id).contents else {
                    unreachable!()
                };
                let index = fields
                    .iter()
                    .position(|(k, _)| k == key)
                    .ok_or("dictionary key missing")?;
                fields.remove(index);
            }
            Mutation::Pop { target, index } => {
                let id = self.target(*target)?;
                let len = self.sequence(id).ok_or("pop target must be a list")?.len();
                let count = i64::try_from(len).map_err(|_| "list length exceeds i64")?;
                let index = if *index < 0 {
                    count
                        .checked_add(*index)
                        .ok_or("list index outside range")?
                } else {
                    *index
                };
                let index = usize::try_from(index)
                    .ok()
                    .filter(|i| *i < len)
                    .ok_or("list index outside range")?;
                let removed = self.list_items_mut(id).remove(index);
                if self.records_list_edit(id) {
                    self.journal
                        .entries
                        .push(ArenaUndo::ListRemove(id, index, removed));
                }
            }
            Mutation::Snapshot { key } => {
                let copy = self.objects[self.context.0].clone();
                let id = ObjectId(self.objects.len());
                self.objects.push(copy);
                let Contents::Fields(fields) = &mut self.object_mut(self.context).contents else {
                    unreachable!()
                };
                if let Some((_, value)) = fields.iter_mut().find(|(k, _)| k == key) {
                    *value = ObjectValue::Object(id);
                } else {
                    fields.push((*key, ObjectValue::Object(id)));
                }
            }
        }
        Ok(())
    }
    /// Compile a diagnostic input at the boundary; apply() consumes typed data.
    pub fn compile_mutation(&mut self, value: &Value) -> Result<Mutation, String> {
        let fields = value.as_object().ok_or("mutation must be an object")?;
        let kind = value["kind"].as_str().ok_or("mutation kind missing")?;
        let allowed: &[&str] = match kind {
            "append" => &["kind", "target", "value"],
            "set" => &["kind", "target", "key", "value"],
            "delete" => &["kind", "target", "key"],
            "pop" => &["kind", "target", "index"],
            "snapshot" => &["kind", "key"],
            _ => return Err("unknown aggregate mutation".into()),
        };
        if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err("unknown aggregate mutation field".into());
        }
        let key = |this: &mut Self| {
            value["key"]
                .as_str()
                .map(|key| Arc::make_mut(&mut this.symbols).intern(key))
                .ok_or("mutation key missing".to_string())
        };
        if kind == "snapshot" {
            return Ok(Mutation::Snapshot { key: key(self)? });
        }
        let target = if value["target"] == "context" {
            Target::Context
        } else {
            Target::Alias(
                value["target"]
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or("mutation alias target missing")?,
            )
        };
        self.target(target)?;
        let input = |this: &mut Self| -> Result<Input, String> {
            let raw = value["value"]
                .as_object()
                .ok_or("mutation value must use literal/alias wrapper")?;
            if raw.len() != 1 {
                return Err("mutation value must have one wrapper".into());
            }
            if let Some(value) = raw.get("literal") {
                reject_literal_references(value)?;
                Ok(Input::Literal(Datum::from_json(
                    value,
                    Arc::make_mut(&mut this.symbols),
                )?))
            } else if let Some(index) = raw
                .get("alias")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
            {
                this.target(Target::Alias(index))?;
                Ok(Input::Alias(index))
            } else {
                Err("unknown mutation value wrapper".into())
            }
        };
        Ok(match kind {
            "append" => Mutation::Append {
                target,
                value: input(self)?,
            },
            "set" => Mutation::Set {
                target,
                key: key(self)?,
                value: input(self)?,
            },
            "delete" => Mutation::Delete {
                target,
                key: key(self)?,
            },
            "pop" => Mutation::Pop {
                target,
                index: value
                    .get("index")
                    .map_or(Ok(-1), |v| v.as_i64().ok_or("pop index must be an integer"))?,
            },
            _ => unreachable!(),
        })
    }
    fn expand(&self, value: &ObjectValue, depth: usize) -> Result<Value, String> {
        if depth > 256 {
            return Err("aggregate depth exceeds limit".into());
        }
        Ok(match value {
            ObjectValue::Atom(value) => value.to_json(&self.symbols),
            ObjectValue::Request(id) => Value::String(format!("native-request-{id}")),
            ObjectValue::GeneratedStep(id) => Value::String(format!("native-step-{id}")),
            ObjectValue::Object(id) => match &self.objects[id.0].contents {
                Contents::List(items) => Value::Array(
                    items
                        .iter()
                        .map(|v| self.expand(v, depth + 1))
                        .collect::<Result<_, _>>()?,
                ),
                Contents::Fields(fields) => Value::Object(
                    fields
                        .iter()
                        .map(|(k, v)| {
                            Ok((self.symbols.name(*k).into(), self.expand(v, depth + 1)?))
                        })
                        .collect::<Result<_, String>>()?,
                ),
            },
        })
    }
    pub(crate) fn hash_execution(&self, state: &mut impl std::hash::Hasher) {
        use std::hash::Hash;

        // Allocation order and handles are significant within a native root.
        self.symbols.hash_local(state);
        self.root.hash(state);
        self.objects.hash(state);
        self.layout.fingerprint().hash(state);
        self.context.hash(state);
        self.physical_copies.hash(state);
        let mut fields: Vec<_> = self.physical_fields.iter().collect();
        fields.sort_unstable_by_key(|(id, _)| **id);
        fields.hash(state);
    }

    pub fn compact_state(&self) -> Result<Value, String> {
        self.expand(&self.root, 0)
    }
    pub fn boundary(&self, source: &StateEnvelope) -> Result<StateEnvelope, String> {
        if source.instances.len() != self.base_instances {
            return Err("arena physical base table mismatch".into());
        }
        let mut result = source.clone();
        for copy in &self.physical_copies {
            let mut instance = source
                .instances
                .get(copy.base as usize)
                .ok_or("physical copy base outside table")?
                .clone();
            for (key, value) in &copy.fields {
                match value {
                    Some(value) => {
                        instance
                            .fields
                            .insert(self.symbols.name(*key).into(), value.to_json(&self.symbols));
                    }
                    None => {
                        instance.fields.remove(self.symbols.name(*key));
                    }
                }
            }
            instance.equality = copy.equality.clone();
            result.instances.push(instance);
        }
        for (id, fields) in &self.physical_fields {
            let instance = result
                .instances
                .get_mut(*id as usize)
                .ok_or("physical override outside table")?;
            for (key, value) in fields {
                match value {
                    Some(value) => {
                        instance
                            .fields
                            .insert(self.symbols.name(*key).into(), value.to_json(&self.symbols));
                    }
                    None => {
                        instance.fields.remove(self.symbols.name(*key));
                    }
                }
            }
        }
        result.state = self.compact_state()?;
        let (bindings, keys) = self.bindings_and_keys()?;
        result.object_bindings = bindings;
        result.board_keys = keys;
        result.validate()?;
        Ok(result)
    }
    /// Sparse (binding index, arena object ID) diagnostic pairs for generated
    /// steps. Copies retain step_id values but have different model identities.
    pub fn generated_step_objects(
        &self,
        bindings: &[ObjectBinding],
    ) -> Result<Vec<(usize, usize)>, String> {
        let step_id = self.symbols.lookup("step_id");
        let mut result = Vec::new();
        for (index, binding) in bindings.iter().enumerate() {
            if binding.kind != ObjectKind::Step {
                continue;
            }
            let ObjectValue::Object(object) =
                self.value_at_path(binding.paths.first().ok_or("step binding path missing")?)?
            else {
                return Err("step binding object missing".into());
            };
            if step_id
                .and_then(|key| self.field(object, key))
                .is_some_and(|value| matches!(value, ObjectValue::GeneratedStep(_)))
            {
                result.push((index, object.0));
            }
        }
        Ok(result)
    }
    pub fn object_keys(&self) -> Result<Vec<Option<Vec<String>>>, String> {
        let bindings = self.object_bindings()?;
        let compact = self.compact_state()?;
        bindings
            .iter()
            .map(|binding| {
                Ok(crate::codec::object_at_path(&compact, &binding.paths[0])?
                    .as_object()
                    .map(|fields| fields.keys().cloned().collect()))
            })
            .collect()
    }
    pub fn object_bindings(&self) -> Result<Vec<ObjectBinding>, String> {
        Ok(self.bindings_and_keys()?.0)
    }
    fn bindings_and_keys(
        &self,
    ) -> Result<(Vec<ObjectBinding>, Option<Vec<crate::codec::BoardKey>>), String> {
        let mut occurrences: Vec<Vec<Vec<ObjectPathPart>>> = vec![Vec::new(); self.objects.len()];
        self.visit(&self.root, &mut Vec::new(), &mut occurrences, 0)?;
        let keys = self.layout.board_keys.as_ref().map(|keys| {
            keys.iter()
                .map(|(hex, id)| crate::codec::BoardKey {
                    hex: *hex,
                    path: occurrences[id.0].first().cloned(),
                })
                .collect()
        });
        // Source capture emits aliases in first-occurrence traversal order.
        let mut order = Vec::new();
        self.order(&self.root, &mut HashSet::new(), &mut order);
        let mut result = Vec::new();
        for id in order {
            if self.objects[id.0].tracked && !occurrences[id.0].is_empty() {
                result.push(ObjectBinding {
                    kind: self.objects[id.0].kind,
                    deepcopy_shared: self.objects[id.0].deepcopy_shared,
                    paths: std::mem::take(&mut occurrences[id.0]),
                });
            }
        }
        Ok((result, keys))
    }

    fn visit(
        &self,
        value: &ObjectValue,
        path: &mut Vec<ObjectPathPart>,
        occurrences: &mut [Vec<Vec<ObjectPathPart>>],
        depth: usize,
    ) -> Result<(), String> {
        if depth > 256 {
            return Err("aggregate depth exceeds limit".into());
        }
        let ObjectValue::Object(id) = value else {
            return Ok(());
        };
        occurrences[id.0].push(path.clone());
        match &self.objects[id.0].contents {
            Contents::List(items) => {
                for (index, value) in items.iter().enumerate() {
                    path.push(ObjectPathPart::Index(index));
                    self.visit(value, path, occurrences, depth + 1)?;
                    path.pop();
                }
            }
            Contents::Fields(fields) => {
                for (key, value) in fields {
                    path.push(ObjectPathPart::Key(self.symbols.name(*key).into()));
                    self.visit(value, path, occurrences, depth + 1)?;
                    path.pop();
                }
            }
        }
        Ok(())
    }
    fn order(&self, value: &ObjectValue, seen: &mut HashSet<ObjectId>, order: &mut Vec<ObjectId>) {
        let ObjectValue::Object(id) = value else {
            return;
        };
        if !seen.insert(*id) {
            return;
        }
        order.push(*id);
        match &self.objects[id.0].contents {
            Contents::List(items) => {
                for value in items {
                    self.order(value, seen, order);
                }
            }
            Contents::Fields(fields) => {
                for (_, value) in fields {
                    self.order(value, seen, order);
                }
            }
        }
    }
}

pub fn aggregate_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    let mut arena = ObjectArena::compile(&envelope)?;
    let operations = operations
        .iter()
        .map(|op| arena.compile_mutation(op))
        .collect::<Result<Vec<_>, _>>()?;
    let mut records = Vec::new();
    for operation in &operations {
        let error = arena.apply(operation).err().map(|_| "invalid_operation");
        records.push(
            json!({"semantic_state": envelope.expand(&arena.compact_state()?)?,
            "object_bindings": arena.object_bindings()?, "object_keys": arena.object_keys()?, "error": error}),
        );
    }
    Ok(json!({"records": records}))
}

fn reject_literal_references(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            if fields.contains_key("$goa2_instance") {
                return Err("physical references are reserved in literal input".into());
            }
            for value in fields.values() {
                reject_literal_references(value)?;
            }
        }
        Value::Array(items) => {
            for value in items {
                reject_literal_references(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod equality_metadata_tests {
    use super::*;
    #[test]
    fn physical_deepcopy_preserves_private_equality_metadata() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/card-equality.json");
        let data: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(path).unwrap(),
        ))
        .unwrap();
        let case = &data["cases"][0];
        let source: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let original = case["queries"][0]["a"].as_u64().unwrap() as u32;
        let mut arena = ObjectArena::compile(&source).unwrap();
        let (copy, _) = arena
            .deep_copy(&ObjectValue::Atom(Datum::Instance(original)))
            .unwrap();
        let ObjectValue::Atom(Datum::Instance(copied)) = copy else {
            panic!("physical copy missing");
        };
        assert_ne!(original, copied);
        let key = Arc::make_mut(&mut arena.symbols).intern("equality_copy");
        arena.set_context(key, copy).unwrap();
        let mut output = arena.boundary(&source).unwrap();
        assert_eq!(
            output.instances[original as usize].equality,
            output.instances[copied as usize].equality
        );
        output.object_bindings.clear();
        let mut symbols = Symbols::default();
        let world = crate::world::NativeWorld::compile(&output, &mut symbols).unwrap();
        assert!(
            world
                .card_model_equal(crate::cards::CardId(original), crate::cards::CardId(copied))
                .unwrap()
        );
    }
}

#[cfg(test)]
mod protection_tests {
    use super::*;
    fn graph() -> ObjectArena {
        let mut symbols = Symbols::default();
        let keys = [
            "execution_context",
            "world",
            "execution_stack",
            "value",
            "pending_input",
            "expire_effect_ids",
            "tied_hero_ids",
            "processed_effect_ids",
            "steps_template",
            "wrapped",
        ]
        .map(|name| symbols.intern(name));
        let object = |kind, contents| Object {
            kind,
            tracked: true,
            deepcopy_shared: false,
            contents,
        };
        let list = |ids: &[usize]| {
            Contents::List(
                ids.iter()
                    .map(|id| ObjectValue::Object(ObjectId(*id)))
                    .collect(),
            )
        };
        let fields = |pairs: &[(usize, usize)]| {
            Contents::Fields(
                pairs
                    .iter()
                    .map(|(key, id)| (keys[*key], ObjectValue::Object(ObjectId(*id))))
                    .collect(),
            )
        };
        ObjectArena {
            symbols: Arc::new(symbols),
            objects: vec![
                object(ObjectKind::Model, fields(&[(0, 1), (1, 2), (2, 3)])),
                object(ObjectKind::Dict, fields(&[(1, 2), (3, 4), (4, 5)])),
                object(ObjectKind::List, list(&[6, 6, 2])),
                object(
                    ObjectKind::Step,
                    fields(&[(8, 7), (3, 4), (4, 5), (5, 8), (6, 9), (7, 10)]),
                ),
                object(ObjectKind::List, list(&[12])),
                object(ObjectKind::Dict, fields(&[(9, 12)])),
                object(ObjectKind::Model, fields(&[(9, 11)])),
                object(ObjectKind::List, list(&[6])),
                object(ObjectKind::List, list(&[12])),
                object(ObjectKind::List, list(&[12])),
                object(ObjectKind::List, list(&[12])),
                object(ObjectKind::List, list(&[6])),
                object(ObjectKind::List, list(&[])),
            ]
            .into_iter()
            .map(Arc::new)
            .collect(),
            layout: Arc::default(),
            root: ObjectValue::Object(ObjectId(0)),
            context: ObjectId(1),
            base_instances: 0,
            base_equality: Arc::from([]),
            physical_copies: Vec::new(),
            physical_fields: HashMap::new(),
            journal: ArenaJournal::default(),
        }
    }
    #[test]
    fn forked_arena_pages_survive_nested_rollback_across_page_boundaries() {
        let mut arena = graph();
        let template = arena.objects[0].clone();
        for _ in arena.objects.len()..ObjectPages::PAGE_LEN - 1 {
            arena.objects.push(template.clone());
        }
        let slot = ObjectId(ObjectPages::PAGE_LEN - 2);
        let hash = |arena: &ObjectArena| {
            use std::hash::{DefaultHasher, Hasher};
            let mut state = DefaultHasher::new();
            arena.hash_execution(&mut state);
            state.finish()
        };
        let original = hash(&arena);

        let fork = arena.with_rollback(|arena| {
            arena.object_mut(slot).tracked = false;
            arena.objects.push(template.clone());
            let checkpoint = arena.begin_rollback();
            arena.object_mut(slot).deepcopy_shared = true;
            arena.objects.push(template);
            arena.commit_rollback(checkpoint);
            arena.clone()
        });

        assert!(Arc::ptr_eq(&arena.layout, &fork.layout));
        assert_eq!(hash(&arena), original);
        assert_ne!(hash(&fork), original);
        for page in arena.objects.pages.iter().chain(&fork.objects.pages) {
            assert_eq!(page.fingerprint.get(), Some(&page.recompute_fingerprint()));
        }
        assert!(arena.objects[slot.0].tracked);
        assert!(!fork.objects[slot.0].tracked);
        assert!(fork.objects[slot.0].deepcopy_shared);
        assert_eq!(fork.objects.len(), ObjectPages::PAGE_LEN + 1);
        assert!(arena.objects.get(ObjectPages::PAGE_LEN - 1).is_none());
        assert!(fork.objects.get(ObjectPages::PAGE_LEN).is_some());
    }

    #[test]
    fn nested_arena_rollback_restores_aliases_order_and_physical_copies() {
        let mut arena = graph();
        arena.base_instances = 1;
        arena.base_equality = Arc::from([None]);
        let key = arena.symbols.lookup("value").unwrap();
        let live_list = ObjectId(4);
        arena.set_physical_field(0, key, Datum::Int(4)).unwrap();
        let before = format!("{arena:?}");

        let result: Result<(), &str> = arena.with_rollback(|arena| {
            let temporary_key = Arc::make_mut(&mut arena.symbols).intern("temporary");
            arena.remove_context(key).unwrap();
            arena
                .set_context(temporary_key, ObjectValue::Object(live_list))
                .unwrap();
            arena
                .append(live_list, ObjectValue::Atom(Datum::Int(2)))
                .unwrap();
            arena
                .set_index(live_list, 0, ObjectValue::Atom(Datum::Int(1)))
                .unwrap();
            arena.set_physical_field(0, key, Datum::Int(6)).unwrap();
            arena.set_physical_field(0, key, Datum::Int(7)).unwrap();
            assert_eq!(arena.journal.entries.len(), 4);
            let (copy, _) = arena
                .deep_copy(&ObjectValue::Atom(Datum::Instance(0)))
                .unwrap();
            let ObjectValue::Atom(Datum::Instance(copy)) = copy else {
                unreachable!()
            };
            let outer = format!("{:?}", arena.objects);
            arena.with_rollback(|arena| {
                arena.clear_list(live_list).unwrap();
                arena.remove_physical_field(copy, key).unwrap();
                arena.remove_physical_field(0, key).unwrap();
                arena.set_physical_field(0, key, Datum::Int(8)).unwrap();
                arena.clear_dictionary(arena.context).unwrap();
            });
            assert_eq!(format!("{:?}", arena.objects), outer);
            assert_eq!(
                arena.physical_override(copy, key),
                Some(Some(&Datum::Int(7)))
            );
            assert_eq!(arena.physical_override(0, key), Some(Some(&Datum::Int(7))));
            arena.set_physical_field(0, key, Datum::Int(9)).unwrap();
            assert_eq!(arena.journal.entries.len(), 4);
            let fork = arena.clone();
            assert!(fork.journal.entries.is_empty());
            assert_eq!(fork.journal.recorded.capacity(), 0);
            assert!(fork.journal.existing_objects.is_none());

            // A child may snapshot a list after the parent has recorded edits.
            // On rollback the snapshot must be restored before those edits.
            let child = arena.begin_rollback();
            arena
                .apply(&Mutation::Pop {
                    target: Target::Object(live_list),
                    index: -2,
                })
                .unwrap();
            arena.clear_list(live_list).unwrap();
            arena
                .append(live_list, ObjectValue::Atom(Datum::Int(10)))
                .unwrap();
            arena.commit_rollback(child);
            arena.list_pop(live_list).unwrap();
            assert!(arena.sequence(live_list).unwrap().is_empty());
            Err("cancel")
        });
        assert_eq!(result, Err("cancel"));
        assert_eq!(format!("{arena:?}"), before);
    }

    #[test]
    fn arena_rollback_runs_during_unwind() {
        let mut arena = graph();
        let before = format!("{arena:?}");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            arena.with_rollback(|arena| {
                arena.list_pop(ObjectId(4)).unwrap();
                arena.list(vec![]);
                panic!("speculation interrupted");
            });
        }));
        assert!(result.is_err());
        assert_eq!(format!("{arena:?}"), before);
    }

    #[test]
    fn bitmap_protection_spans_words_and_copies_remain_private() {
        let mut root = ProtectedObjects::new(130);
        for id in [0, 63, 64, 127, 129] {
            assert!(root.insert::<true>(ObjectId(id)));
            assert!(!root.insert::<true>(ObjectId(id)));
            assert!(root.contains(&ObjectId(id)));
        }
        assert!(!root.contains(&ObjectId(130)));
        let mut worker = root.clone();
        worker.insert::<true>(ObjectId(65));
        assert!(!root.contains(&ObjectId(65)));
        assert!(worker.contains(&ObjectId(65)));
        assert_eq!(root.into_iter().count(), 5);
    }
    #[test]
    fn fresh_context_proof_rechecks_rewritten_and_new_pages_after_rollback() {
        let mut arena = graph();
        assert!(arena.context_edges_are_private(0, None));
        let initial = arena.objects.len();
        let revision = arena.objects.revision;
        Arc::get_mut(&mut arena.layout).unwrap().private_context = Some((initial, revision));
        let ObjectValue::Object(private) = arena.list(Vec::new()) else {
            unreachable!()
        };
        let key = arena.symbols.lookup("wrapped").unwrap();
        arena
            .set_context(key, ObjectValue::Object(private))
            .unwrap();
        assert!(arena.fresh_context_object_is_private(private));
        assert!(!arena.fresh_context_object_is_private(ObjectId(4)));

        arena.with_rollback(|arena| {
            arena
                .append(ObjectId(2), ObjectValue::Object(private))
                .unwrap();
            assert!(!arena.fresh_context_object_is_private(private));
            assert!(arena.readonly_objects().contains(&private));
        });
        assert!(arena.fresh_context_object_is_private(private));

        arena.with_rollback(|arena| {
            let external = arena.list(vec![ObjectValue::Object(arena.context_id())]);
            arena
                .set_root(arena.symbols.lookup("world").unwrap(), external)
                .unwrap();
            assert!(!arena.fresh_context_object_is_private(private));
            assert!(arena.readonly_objects().contains(&private));
        });
        assert!(arena.fresh_context_object_is_private(private));
    }

    #[test]
    fn cycle_checks_follow_deferred_siblings_past_existing_cycles() {
        let mut arena = graph();
        let cyclic = ObjectValue::Object(ObjectId(2));
        let leaf = ObjectValue::Object(ObjectId(12));
        let root = arena.list(vec![leaf, cyclic]);
        assert!(arena.reaches(&root, ObjectId(12)));
        assert!(!arena.reaches(&root, ObjectId(5)));
        assert!(arena.append(ObjectId(12), root).is_err());
        assert!(arena.sequence(ObjectId(12)).unwrap().is_empty());
    }

    #[test]
    fn readonly_walk_preserves_live_payload_exclusions_aliases_and_cycles() {
        let arena = graph();
        let membership = arena.readonly_objects();
        assert!(membership.objects.is_empty());
        let protected: HashSet<_> = (0..arena.objects.len())
            .map(ObjectId)
            .filter(|id| membership.contains(id))
            .collect();
        assert_eq!(
            protected,
            [2, 3, 6, 7, 11].map(ObjectId).into_iter().collect()
        );
        let step = arena
            .readonly_step_objects(ObjectId(3))
            .into_iter()
            .collect::<HashSet<_>>();
        assert_eq!(step, [3, 6, 7, 11].map(ObjectId).into_iter().collect());
        // Identically named fields on non-step containers remain traversable.
        let context = arena
            .readonly_step_objects(arena.context)
            .into_iter()
            .collect::<HashSet<_>>();
        assert_eq!(
            context,
            [1, 2, 4, 5, 6, 11, 12].map(ObjectId).into_iter().collect()
        );
    }
    #[test]
    fn refreshed_roots_detach_old_world_lists_without_protecting_context_aliases() {
        let mut arena = graph();
        let old = arena.readonly_objects();
        let ObjectValue::Object(new) = arena.list(Vec::new()) else {
            unreachable!()
        };
        assert!(!old.contains(&new));
        let world = arena.symbols.lookup("world").unwrap();
        arena.set_root(world, ObjectValue::Object(new)).unwrap();
        let updated = arena.readonly_objects();
        assert!(!updated.contains(&ObjectId(2)));
        assert!(updated.contains(&new));
        assert!(updated.contains(&ObjectId(6)));
        assert!(old.contains(&ObjectId(2)));
        assert!(!updated.contains(&arena.context));
    }
}
