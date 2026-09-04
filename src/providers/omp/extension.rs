use camino::Utf8PathBuf;

use crate::providers::ManagedArtifact;

/// Embedded plain JavaScript autocomplete extension for OMP.
///
/// OMP discovers `.omp/extensions/*.js` and loads each one **in-process** as
/// an ES module whose default export is a factory receiving the extension API
/// (`ExtensionAPI`, documented in `@oh-my-pi/pi-coding-agent`).
///
/// The extension registers an autocomplete provider via `ctx.ui.addAutocompleteProvider`
/// on `session_start`. When the user is typing the argument for any shipped `/ivar-*`
/// command that accepts an existing feature, it provides completion candidates by
/// querying `ivar feature list --json`.
///
/// In a session bound to a feature (`IVAR_FEATURE` is set), or for lines not matching
/// the targeted commands, it delegates entirely to the live `current` provider.
/// Every method (both required and optional) is faithfully forwarded to `current`
/// so standard file and command completions continue uninterrupted.
pub const OMP_EXTENSION: &str = r#"// ivar autocomplete extension for OMP
// Materialised by `ivar sync`. Do not edit.

import { execFileSync } from "node:child_process";

const TARGET_COMMANDS = [
  "/ivar-connect",
  "/ivar-promote",
  "/ivar-deliver",
  "/ivar-feature-status",
  "/ivar-feature-cleanup",
  "/ivar-plan",
  "/ivar-review",
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
        relative_path: Utf8PathBuf::from(".omp/extensions/ivar.js"),
        contents: OMP_EXTENSION,
    }]
}
