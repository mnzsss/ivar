//! The cross-packet rule: a task that consumes an interface waits for the
//! task that produces it.
//!
//! `plan.md` carries two tables. The wave tables say which task is blocked by
//! which (`| Task | Points | Blocked by | Outcome |`); the Interfaces table
//! locks each signature before any packet is written
//! (`| Signature | Produced by | Consumed by |`). A packet's own
//! `Consumes:` bullet counts as a consumer too, so a signature a packet cites
//! without the table listing it still has to be waited for.

use std::collections::{BTreeMap, BTreeSet};

use super::packet::{Line, consumes_text, lines};
use super::{Finding, MISSING_DEPENDENCY, PacketSource};

struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn cells(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn is_separator(row: &[String]) -> bool {
    row.iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':' | ' ')))
}

/// Every pipe table outside a fenced block.
fn tables(plan: &str) -> Vec<Table> {
    let all = lines(plan);
    let mut found = Vec::new();
    let mut block: Vec<&Line<'_>> = Vec::new();
    for line in all.iter().map(Some).chain([None]) {
        match line {
            Some(line) if !line.fenced && line.text.trim_start().starts_with('|') => {
                block.push(line);
            }
            _ => {
                if let Some((head, rest)) = block.split_first() {
                    found.push(Table {
                        header: cells(head.text),
                        rows: rest
                            .iter()
                            .map(|line| cells(line.text))
                            .filter(|row| !is_separator(row))
                            .collect(),
                    });
                }
                block.clear();
            }
        }
    }
    found
}

/// Task numbers named in `cell`: `1`, `Task 2`, `` `tasks/03-name.md` ``.
fn numbers(cell: &str) -> Vec<u32> {
    cell.split([',', ';', ' '])
        .filter_map(|token| {
            let name = token.trim_matches('`').rsplit('/').next()?.to_lowercase();
            let rest = name
                .trim_start_matches("task")
                .trim_start_matches(['t', '#', '-']);
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect()
}

fn column(header: &[String], name: &str) -> Option<usize> {
    header
        .iter()
        .position(|cell| cell.to_lowercase().contains(name))
}

/// Task number → the tasks its wave table lists in `Blocked by`.
fn blockers(tables: &[Table]) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut out: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for table in tables {
        let Some(task) = column(&table.header, "task") else {
            continue;
        };
        let Some(blocked) = column(&table.header, "blocked by") else {
            continue;
        };
        for row in &table.rows {
            let Some(id) = row
                .get(task)
                .and_then(|cell| numbers(cell).first().copied())
            else {
                continue;
            };
            let waits = row
                .get(blocked)
                .map(|cell| numbers(cell))
                .unwrap_or_default();
            out.entry(id).or_default().extend(waits);
        }
    }
    out
}

/// Every task `task` waits for, directly or through others.
fn waits_for(task: u32, blockers: &BTreeMap<u32, BTreeSet<u32>>) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![task];
    while let Some(current) = pending.pop() {
        for next in blockers.get(&current).into_iter().flatten() {
            if seen.insert(*next) {
                pending.push(*next);
            }
        }
    }
    seen
}

fn normalize(signature: &str) -> String {
    signature
        .replace('`', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

struct Interface {
    signature: String,
    producers: BTreeSet<u32>,
    consumers: BTreeSet<u32>,
}

fn interfaces(tables: &[Table]) -> Vec<Interface> {
    let mut out = Vec::new();
    for table in tables {
        let (Some(produced), Some(consumed)) = (
            column(&table.header, "produced by"),
            column(&table.header, "consumed by"),
        ) else {
            continue;
        };
        for row in &table.rows {
            let signature = row.first().map(|cell| normalize(cell)).unwrap_or_default();
            let ids = |at: usize| -> BTreeSet<u32> {
                row.get(at)
                    .map(|cell| numbers(cell))
                    .unwrap_or_default()
                    .into_iter()
                    .collect()
            };
            if !signature.is_empty() {
                out.push(Interface {
                    signature,
                    producers: ids(produced),
                    consumers: ids(consumed),
                });
            }
        }
    }
    out
}

pub(super) fn check(plan: &str, packets: &[PacketSource]) -> Vec<Finding> {
    let tables = tables(plan);
    let mut interfaces = interfaces(&tables);
    if interfaces.is_empty() {
        return Vec::new();
    }
    let blockers = blockers(&tables);
    let owner: BTreeMap<u32, &str> = packets
        .iter()
        .filter_map(|packet| {
            Some((
                numbers(&packet.file).first().copied()?,
                packet.file.as_str(),
            ))
        })
        .collect();

    for packet in packets {
        let Some(number) = numbers(&packet.file).first().copied() else {
            continue;
        };
        let consumed = normalize(&consumes_text(&packet.text));
        for interface in &mut interfaces {
            if consumed.contains(&interface.signature) {
                interface.consumers.insert(number);
            }
        }
    }

    let mut findings = Vec::new();
    for interface in &interfaces {
        for &consumer in &interface.consumers {
            let waits = waits_for(consumer, &blockers);
            for &producer in &interface.producers {
                if producer == consumer || waits.contains(&producer) {
                    continue;
                }
                findings.push(Finding {
                    file: owner.get(&consumer).map_or("plan.md", |file| file).to_owned(),
                    line: 0,
                    code: MISSING_DEPENDENCY,
                    what: format!(
                        "task {consumer} consumes `{}` produced by task {producer} but does not wait for it: add {producer} to its `Blocked by`",
                        interface.signature
                    ),
                });
            }
        }
    }
    findings
}
