import assert from "node:assert/strict";
import { test } from "node:test";
import {
  entriesUrlFromLibrariumPathname,
  librariumTargetFromPathname,
} from "./entriesPath.ts";

test("opens a nested markdown path as preview, not entries root", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname(
      "/projects/turret/design/sas-architecture-tracker.md",
      true,
    ),
    "/entries/projects/turret/design?open=%2Fprojects%2Fturret%2Fdesign%2Fsas-architecture-tracker.md",
  );
});

test("opens a root-level document", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/notes.md", true),
    "/entries?open=%2Fnotes.md",
  );
});

test("opens a directory path as a directory", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/projects/turret/design", false),
    "/entries/projects/turret/design",
  );
});

test("maps empty path to entries root", () => {
  assert.equal(entriesUrlFromLibrariumPathname("/", false), "/entries");
  assert.equal(entriesUrlFromLibrariumPathname("/", true), "/entries");
});

test("opens a hidden file as preview", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/secrets/.gitignore", true),
    "/entries/secrets?open=%2Fsecrets%2F.gitignore",
  );
});

test("opens a version-like directory name as a directory", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/releases/v0.1/", false),
    "/entries/releases/v0.1",
  );
});

test("opens an extensionless document as preview", () => {
  assert.equal(
    entriesUrlFromLibrariumPathname("/projects/Makefile", true),
    "/entries/projects?open=%2Fprojects%2FMakefile",
  );
});

test("splits a librarium path into parent and name", () => {
  assert.deepEqual(librariumTargetFromPathname("/releases/v0.1/"), {
    abs: "/releases/v0.1",
    parent: "/releases",
    name: "v0.1",
  });
  assert.deepEqual(librariumTargetFromPathname("/README"), {
    abs: "/README",
    parent: "/",
    name: "README",
  });
  assert.equal(librariumTargetFromPathname("/"), null);
});
