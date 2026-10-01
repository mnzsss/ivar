use camino::Utf8Path;

use crate::domain::provider::Provider;
use crate::providers::ManagedArtifact;

/// Embedded plain JavaScript extension for OMP.
///
/// OMP discovers `.omp/extensions/*.js` and loads each one **in-process** as
/// an ES module whose default export is a factory receiving the extension API
/// (`ExtensionAPI`, documented in `@oh-my-pi/pi-coding-agent`).
///
/// It does two things:
///
/// - **Registers the shipped `/ivar-*` commands** from the `.omp/commands/`
///   next to its own real path, via `pi.registerCommand`. OMP's file-command
///   discovery skips gitignored files, and `ivar sync` gitignores
///   `.omp/commands/ivar-*.md`; extension commands bypass that discovery and
///   take precedence over file commands of the same name. Each invocation
///   re-reads its file, strips the frontmatter, substitutes `$ARGUMENTS`
///   (or appends the arguments when the body has none) and sends the result
///   with `pi.sendUserMessage`.
/// - **Completes feature names** for the `/ivar-*` commands that take an
///   existing feature, from `ivar feature list --json`, delegating every
///   other line (and every line in a feature-bound session) to the live
///   `current` provider.
pub const OMP_EXTENSION: &str = r#"// ivar extension for OMP: /ivar-* commands and feature-name autocomplete
// Materialised by `ivar sync`. Do not edit.

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const COMMAND_FILE = /^ivar-[a-z0-9-]+\.md$/;

// Returns { meta, body }, or null when a frontmatter block is opened but never closed.
function splitFrontmatter(raw) {
  if (!/^---\r?\n/.test(raw)) return { meta: {}, body: raw };
  const match = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/.exec(raw);
  if (!match) return null;
  const meta = {};
  for (const line of match[1].split(/\r?\n/)) {
    const colon = line.indexOf(":");
    if (colon > 0) meta[line.slice(0, colon).trim()] = line.slice(colon + 1).trim();
  }
  return { meta, body: raw.slice(match[0].length) };
}

function expandArguments(body, args) {
  const text = String(args ?? "").trim();
  if (body.includes("$ARGUMENTS")) return body.replaceAll("$ARGUMENTS", () => text);
  return text ? `${body.trimEnd()}\n\n${text}` : body;
}

function registerCommands(pi) {
  if (typeof pi.registerCommand !== "function" || typeof pi.sendUserMessage !== "function") {
    return;
  }
  const dir = fileURLToPath(new URL("../commands/", import.meta.url));
  let files;
  try {
    files = readdirSync(dir).filter((name) => COMMAND_FILE.test(name)).sort();
  } catch (_err) {
    return;
  }
  for (const file of files) {
    const path = join(dir, file);
    let parsed;
    try {
      parsed = splitFrontmatter(readFileSync(path, "utf-8"));
    } catch (_err) {
      continue;
    }
    if (!parsed) continue;
    try {
      pi.registerCommand(file.slice(0, -".md".length), {
        description: parsed.meta.description || undefined,
        handler: async (args, ctx) => {
          let current;
          try {
            current = splitFrontmatter(readFileSync(path, "utf-8"));
          } catch (_err) {
            current = null;
          }
          if (!current) {
            ctx?.ui?.notify?.(`ivar: could not read ${path}; run \`ivar sync\``, "error");
            return;
          }
          pi.sendUserMessage(expandArguments(current.body, args));
        },
      });
    } catch (_err) {
      // A rejected registration (e.g. a name clash) must not stop the others.
    }
  }
}

const TARGET_COMMANDS = [
  "/ivar-connect",
  "/ivar-promote",
  "/ivar-deliver",
  "/ivar-feature-status",
  "/ivar-feature-cleanup",
  "/ivar-plan",
  "/ivar-review",
  "/ivar-workspace",
];

function getFeatureCandidates() {
  try {
    const raw = execFileSync("ivar", ["feature", "list", "--json"], {
      encoding: "utf-8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 1500,
    });
    const parsed = JSON.parse(raw);
    const features = Array.isArray(parsed?.features) ? parsed.features : [];
    return features.map((f) => {
      const name = String(f?.name ?? "");
      const state = f?.state ? String(f.state) : "";
      const repos = Array.isArray(f?.repos) ? f.repos.join(", ") : "";
      let description = state;
      if (repos) {
        description = description ? `${description} (${repos})` : repos;
      }
      return {
        value: name,
        label: name,
        description: description || undefined,
      };
    }).filter((item) => item.value.length > 0);
  } catch (_err) {
    return [];
  }
}

function matchTargetCommand(lineBeforeCursor) {
  for (const cmd of TARGET_COMMANDS) {
    const prefix = `${cmd} `;
    if (lineBeforeCursor.startsWith(prefix)) {
      const argPrefix = lineBeforeCursor.slice(prefix.length);
      // Only complete the first positional argument (no whitespace yet)
      if (!/\s/.test(argPrefix)) {
        return { matched: true, argPrefix, cmdPrefix: prefix };
      }
      return { matched: false };
    }
  }
  return { matched: false };
}

export default function ivarExtension(pi) {
  registerCommands(pi);
  pi.on("session_start", async (_event, ctx) => {
    if (!ctx?.ui?.addAutocompleteProvider) {
      return;
    }

    ctx.ui.addAutocompleteProvider((current) => {
      const provider = {
        async getSuggestions(lines, cursorLine, cursorCol, signal) {
          try {
            if (process.env.IVAR_FEATURE) {
              return current?.getSuggestions
                ? await current.getSuggestions(lines, cursorLine, cursorCol, signal)
                : null;
            }

            const currentLine = lines[cursorLine] || "";
            const beforeCursor = currentLine.slice(0, cursorCol);
            const match = matchTargetCommand(beforeCursor);

            if (match.matched) {
              const candidates = getFeatureCandidates();
              const lowerArg = match.argPrefix.toLowerCase();
              const filtered = candidates.filter((item) =>
                item.value.toLowerCase().startsWith(lowerArg)
              );

              return {
                items: filtered,
                prefix: match.argPrefix,
              };
            }
          } catch (_err) {
            // Degrade silently to standard provider
          }

          return current?.getSuggestions
            ? await current.getSuggestions(lines, cursorLine, cursorCol, signal)
            : null;
        },

        applyCompletion(lines, cursorLine, cursorCol, item, prefix) {
          try {
            const currentLine = lines[cursorLine] || "";
            const beforeCursor = currentLine.slice(0, cursorCol);
            const match = matchTargetCommand(beforeCursor);

            if (match.matched) {
              const afterCursor = currentLine.slice(cursorCol);
              const newLine =
                beforeCursor.slice(0, beforeCursor.length - prefix.length) +
                item.value +
                afterCursor;
              const newLines = [...lines];
              newLines[cursorLine] = newLine;
              const newCursorCol =
                cursorCol - prefix.length + item.value.length;

              return {
                lines: newLines,
                cursorLine,
                cursorCol: newCursorCol,
              };
            }
          } catch (_err) {
            // Degrade silently to standard provider
          }

          if (current?.applyCompletion) {
            return current.applyCompletion(lines, cursorLine, cursorCol, item, prefix);
          }

          return { lines, cursorLine, cursorCol };
        },
      };

      if (typeof current?.getInlineHint === "function") {
        provider.getInlineHint = function (lines, cursorLine, cursorCol) {
          return current.getInlineHint(lines, cursorLine, cursorCol);
        };
      }

      if (typeof current?.trySyncSlashCompletion === "function") {
        provider.trySyncSlashCompletion = function (textBeforeCursor) {
          return current.trySyncSlashCompletion(textBeforeCursor);
        };
      }

      if (typeof current?.trySyncInlineReplace === "function") {
        provider.trySyncInlineReplace = function (textBeforeCursor) {
          return current.trySyncInlineReplace(textBeforeCursor);
        };
      }

      if (typeof current?.getForceFileSuggestions === "function") {
        provider.getForceFileSuggestions = function (lines, cursorLine, cursorCol, signal) {
          return current.getForceFileSuggestions(lines, cursorLine, cursorCol, signal);
        };
      }

      if (typeof current?.shouldTriggerFileCompletion === "function") {
        provider.shouldTriggerFileCompletion = function (lines, cursorLine, cursorCol) {
          return current.shouldTriggerFileCompletion(lines, cursorLine, cursorCol);
        };
      }

      return provider;
    });
  });
}
"#;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8Path::new(Provider::OMP_EXTENSIONS_DIR).join("ivar.js"),
        contents: OMP_EXTENSION,
    }]
}
