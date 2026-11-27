//! Compiler-owned ASCII graph metadata only, never message or actor authority.
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: usize = 256;
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub terminal: bool,
    pub edges: Vec<(String, usize)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidTable;

#[derive(Debug)]
pub struct Table {
    initial: usize,
    names: Vec<String>,
    events: Vec<String>,
    rows: Vec<Vec<(usize, usize)>>,
}

fn valid_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    s.len() <= 1024
        && bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

impl Table {
    pub fn verify(initial: usize, nodes: &[Node]) -> Result<Self, InvalidTable> {
        if nodes.is_empty() || nodes.len() > LIMIT || initial >= nodes.len() {
            return Err(InvalidTable);
        }
        let mut names = Vec::<String>::new();
        let mut events = BTreeSet::<String>::new();
        let mut count = 0usize;
        for node in nodes {
            if !valid_name(&node.name)
                || names.last().is_some_and(|p| p >= &node.name)
                || (node.terminal && !node.edges.is_empty())
                || node.edges.len() > LIMIT - count
            {
                return Err(InvalidTable);
            }
            names.push(node.name.clone());
            let mut previous = "";
            for (event, target) in &node.edges {
                if !valid_name(event) || previous >= event.as_str() || *target >= nodes.len() {
                    return Err(InvalidTable);
                }
                count += 1;
                previous = event;
                events.insert(event.clone());
            }
        }
        let mut seen = vec![false; nodes.len()];
        let mut pending = vec![initial];
        while let Some(id) = pending.pop() {
            if seen[id] { continue; }
            seen[id] = true;
            pending.extend(nodes[id].edges.iter().map(|(_, target)| *target));
        }
        if seen.iter().any(|v| !v) { return Err(InvalidTable); }
        let events: Vec<String> = events.into_iter().collect();
        let slots: BTreeMap<&str, usize> =
            events.iter().enumerate().map(|(i, e)| (e.as_str(), i)).collect();
        let rows = nodes.iter().map(|node| node.edges.iter()
            .map(|(event, target)| (slots[event.as_str()], *target)).collect()).collect();
        Ok(Self { initial, names, events, rows })
    }

    pub fn initial(&self) -> usize { self.initial }
    pub fn names(&self) -> &[String] { &self.names }
    pub fn events(&self) -> &[String] { &self.events }

    /// Err is an invalid index, Ok(None) a valid event with no outgoing edge.
    pub fn successor(&self, node: usize, event: usize) -> Result<Option<usize>, InvalidTable> {
        if event >= self.events.len() { return Err(InvalidTable); }
        let row = self.rows.get(node).ok_or(InvalidTable)?;
        Ok(row.iter().find(|(slot, _)| *slot == event).map(|(_, target)| *target))
    }
}
