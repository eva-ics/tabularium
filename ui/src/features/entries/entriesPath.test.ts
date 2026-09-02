import assert from "node:assert/strict";
import { test } from "node:test";
import { entriesUrlFromLibrariumPathname } from "./entriesPath.ts";

test("opens a nested markdown path as preview, not entries root", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname(
      "/projects/turret/design/sas-architecture-tracker.md",
    ),
    "/entries/projects/turret/design?open=%2Fprojects%2Fturret%2Fdesign%2Fsas-architecture-tracker.md",
  );
});

test("opens a root-level document", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/notes.md"),
    "/entries?open=%2Fnotes.md",
  );
});

test("treats a path without a file suffix as a directory", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/projects/turret/design"),
    "/entries/projects/turret/design",
  );
});

test("maps empty path to entries root", () => {
  assert.equal(entriesUrlFromLibrariumPathname("/"), "/entries");
});

test("opens a hidden file as preview", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/secrets/.gitignore"),
    "/entries/secrets?open=%2Fsecrets%2F.gitignore",
  );
});
