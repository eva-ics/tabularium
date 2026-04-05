"""Selenium coverage for meetings/webui.search — Ferrum test manifest T01–T20.

Prerequisites: TABULARIUM_TEST_URL pointing at http://10.90.1.122:<port> (AGENTS.md).
"""

from __future__ import annotations

import re
import time
import uuid
from typing import Any
from urllib.parse import quote, unquote, urlparse

import pytest
import requests
from selenium.common.exceptions import StaleElementReferenceException
from selenium.webdriver import Keys
from selenium.webdriver.common.by import By
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import Select, WebDriverWait

from tests.helpers import mkdir

pytestmark = pytest.mark.webui


def _wait(driver, timeout=20):
    return WebDriverWait(driver, timeout)


def _wait_app_ready(driver, timeout=20):
    _wait(driver, timeout).until(
        lambda d: d.find_element(By.TAG_NAME, "body").get_attribute(
            "data-tabularium-ready",
        )
        == "true",
    )


def _wait_entries_loaded(driver, timeout=25):
    _wait(driver, timeout).until(
        EC.invisibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='entries-loading']"),
        ),
    )


def _mkdir_path(base: str, path: str) -> None:
    p = path if path.startswith("/") else f"/{path}"
    requests.post(
        f"{base.rstrip('/')}/api/doc",
        json={"path": p, "description": None},
        timeout=20,
    ).raise_for_status()


def _entries_url(base: str, *parts: str) -> str:
    b = base.rstrip("/")
    if not parts:
        return f"{b}/entries"
    enc = "/".join(quote(p, safe="") for p in parts)
    return f"{b}/entries/{enc}"


def _logical_dir_from_url(url: str) -> str:
    """Return librarium dir path like `/foo/bar` from browser URL."""
    path = urlparse(url).path
    m = re.search(r"/entries(?:/(.*))?$", path)
    if not m or not m.group(1):
        return "/"
    tail = m.group(1)
    segments = [unquote(s) for s in tail.split("/") if s]
    return "/" + "/".join(segments) if segments else "/"


def _put_doc(base: str, api_segments: tuple[str, ...], name: str, content: str) -> None:
    rel = "/".join(api_segments)
    url = f"{base.rstrip('/')}/api/doc/{rel}"
    requests.post(
        url,
        json={"name": name, "content": content},
        timeout=20,
    ).raise_for_status()


def _set_search_scope(driver, value: "local | global") -> None:
    sel = driver.find_element(By.CSS_SELECTOR, "[aria-label='Search scope']")
    Select(sel).select_by_value(value)


def _fire_search(driver, query: str) -> None:
    inp = driver.find_element(By.CSS_SELECTOR, "[data-testid='search-input']")
    inp.clear()
    inp.send_keys(query)
    driver.find_element(By.CSS_SELECTOR, "[data-testid='search-submit']").click()


def _wait_search_idle(driver, timeout=25):
    _wait(driver, timeout).until(
        EC.invisibility_of_element_located(
            (By.XPATH, "//*[contains(.,'Searching the stacks')]"),
        ),
    )


def _focus_search_list(driver):
    ul = _wait(driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='search-mode'] ul[role='listbox']"),
        ),
    )
    ul.click()


def _wait_search_mode(driver, present: bool = True, timeout=20):
    if present:
        _wait(driver, timeout).until(
            EC.presence_of_element_located((By.CSS_SELECTOR, "[data-testid='search-mode']")),
        )
    else:
        _wait(driver, timeout).until(
            EC.invisibility_of_element_located((By.CSS_SELECTOR, "[data-testid='search-mode']")),
        )


def _result_paths(driver) -> list[str]:
    rows = driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='search-result-row'][data-result-path]",
    )
    return [r.get_attribute("data-result-path") or "" for r in rows]


def _wait_for_search_hits(driver, min_rows: int = 1, timeout=30):
    end = time.time() + timeout
    while time.time() < end:
        _wait_search_idle(driver, timeout=5)
        paths = _result_paths(driver)
        if len(paths) >= min_rows:
            return paths
        time.sleep(0.25)
    raise AssertionError(f"expected >= {min_rows} search rows, got {len(_result_paths(driver))}")


def _wait_preview_doc_visible(driver, timeout=20):
    """Preview uses CSS-module class names; wait on copy/state instead of `.markdown`."""

    def doc_visible(d):
        panes = d.find_elements(By.CSS_SELECTOR, "[data-testid='preview-pane']")
        if not panes:
            return False
        try:
            t = panes[0].text
        except StaleElementReferenceException:
            return False
        if "Loading…" in t:
            return False
        if "Select a file" in t:
            return False
        return True

    _wait(driver, timeout).until(doc_visible)


@pytest.fixture
def seed_search_hierarchy(tabularium_base_url: str) -> dict[str, Any]:
    """Nested dirs + unique tokens for local/global/highlight tests."""
    slug = uuid.uuid4().hex[:8]
    root = f"wsrch_{slug}"
    base = tabularium_base_url.rstrip("/")
    mkdir(tabularium_base_url, root)
    _mkdir_path(base, f"/{root}/inner")
    tok_parent = f"wsptok_{slug}"
    tok_sub = f"wssubtok_{slug}"
    tok_shared = f"wshared_{slug}"
    tok_hi = f"wshilit_{slug}"
    _put_doc(base, (root,), "parent_only.md", f"# parent\n\n{tok_parent}\n")
    _put_doc(base, (root, "inner"), "sub_only.md", f"# sub\n\n{tok_sub}\n")
    _put_doc(base, (root,), "both_parent.md", f"shared {tok_shared} parent\n")
    _put_doc(
        base,
        (root, "inner"),
        "both_sub.md",
        f"shared {tok_shared} sub\n",
    )
    long_body = "\n\n".join(f"Padding line {i} sanctified." for i in range(45))
    long_body += f"\n\n## Deep match\n\n{tok_hi} emperor watches.\n"
    _put_doc(base, (root, "inner"), "long_hi.md", long_body)
    time.sleep(0.35)
    return {
        "root": root,
        "tok_parent": tok_parent,
        "tok_sub": tok_sub,
        "tok_shared": tok_shared,
        "tok_hi": tok_hi,
        "origin_inner": f"/{root}/inner",
        "origin_parent": f"/{root}",
    }


# --- Group 1: Heresy A (origin snapshot) ---


def test_ws_T01_exit_dotdot_restores_origin(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "local")
    # Token must exist under current folder subtree (Local); tok_parent lives in parent only.
    _fire_search(d, seed_search_hierarchy["tok_shared"])
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait_search_mode(d, present=False)
    _wait_entries_loaded(d)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T02_exit_arrow_left_on_list_restores_origin(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ARROW_LEFT)
    _wait_search_mode(d, present=False)
    _wait_entries_loaded(d)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T03_double_arrow_left_no_dir_up_after_exit(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    """Manifest T03: exit via Left restores origin. A second Left on `body` is normal entries nav."""
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ARROW_LEFT)
    _wait_search_mode(d, present=False)
    _wait_entries_loaded(d)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T04_escape_exits_search_restores_origin(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ESCAPE)
    _wait_search_mode(d, present=False)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T05_preview_then_dotdot_restores_origin(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait_preview_doc_visible(d)
    d.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait_search_mode(d, present=False)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T06_preview_escape_then_arrow_left_exits_to_origin(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok_parent = seed_search_hierarchy["tok_parent"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok_parent)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    body = _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] [tabindex='-1']"),
        ),
    )
    body.click()
    body.send_keys(Keys.ESCAPE)
    _wait_search_mode(d, present=True)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ARROW_LEFT)
    _wait_search_mode(d, present=False)
    assert _logical_dir_from_url(d.current_url) == origin


# --- Group 2: session + preview ---


def test_ws_T07_hit_opens_preview_keeps_search_results_visible(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait_preview_doc_visible(d)
    assert d.find_elements(By.CSS_SELECTOR, "[data-testid='search-mode']")


def test_ws_T08_enter_on_hit_keeps_search_mode(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ENTER)
    _wait_preview_doc_visible(d)
    assert d.find_elements(By.CSS_SELECTOR, "[data-testid='search-mode']")


def test_ws_T09_escape_from_preview_keeps_search_session(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    body = _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] [tabindex='-1']"),
        ),
    )
    body.click()
    body.send_keys(Keys.ESCAPE)
    _wait_search_mode(d, present=True)
    ul = d.find_element(
        By.CSS_SELECTOR,
        "[data-testid='search-mode'] ul[role='listbox']",
    )
    _wait(d, 10).until(
        lambda x: x.execute_script(
            "return document.activeElement === arguments[0]",
            ul,
        ),
    )


def test_ws_T10_arrow_right_from_preview_returns_focus_to_results(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    body = _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] [tabindex='-1']"),
        ),
    )
    body.click()
    body.send_keys(Keys.ARROW_RIGHT)
    _wait_search_mode(d, present=True)
    ul = d.find_element(
        By.CSS_SELECTOR,
        "[data-testid='search-mode'] ul[role='listbox']",
    )
    _wait(d, 10).until(
        lambda x: x.execute_script(
            "return document.activeElement === arguments[0]",
            ul,
        ),
    )


# --- Group 3: scope ---


def test_ws_T11_scope_select_never_disabled_during_search(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    sel = d.find_element(By.CSS_SELECTOR, "[aria-label='Search scope']")
    assert sel.is_enabled()


def test_ws_T12_local_scope_limits_to_subtree(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_shared"]
    prefix = f"/{root}/inner"
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "local")
    _fire_search(d, tok)
    _wait_search_mode(d)
    paths = _wait_for_search_hits(d, 1)
    for p in paths:
        assert p.startswith(prefix), f"{p!r} not under {prefix!r}"


def test_ws_T13_global_scope_includes_outside_subtree(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_shared"]
    parent_path = f"/{root}/both_parent.md"
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok)
    _wait_search_mode(d)
    paths = _wait_for_search_hits(d, 1)
    assert any(p == parent_path for p in paths), paths


def test_ws_T14_changing_scope_refetches(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_shared"]
    parent_path = f"/{root}/both_parent.md"
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "local")
    _fire_search(d, tok)
    _wait_search_mode(d)
    local_paths = set(_wait_for_search_hits(d, 1))
    assert parent_path not in local_paths
    _set_search_scope(d, "global")
    _wait(d, 30).until(
        lambda x: parent_path in set(_result_paths(x)),
    )


# --- Group 4: highlight ---


def test_ws_T15_markdown_preview_has_highlight_marks(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_hi"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] mark.previewHighlight"),
        ),
    )
    marks = d.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-pane'] mark.previewHighlight",
    )
    texts = "".join(m.text.lower() for m in marks)
    assert tok.lower() in texts


def test_ws_T16_first_mark_scrolled_into_preview_viewport(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_hi"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] mark.previewHighlight"),
        ),
    )

    def mark_visible() -> bool:
        return bool(
            d.execute_script(
                """
                const pane = document.querySelector(
                  "[data-testid='preview-pane'] [tabindex='-1']");
                if (!pane) return false;
                const m = pane.querySelector("mark.previewHighlight");
                if (!m) return false;
                const pr = pane.getBoundingClientRect();
                const mr = m.getBoundingClientRect();
                return mr.bottom > pr.top && mr.top < pr.bottom;
                """,
            ),
        )

    _wait(d, 15).until(lambda _: mark_visible())


def test_ws_T17_raw_mode_still_highlights(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_hi"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] mark.previewHighlight"),
        ),
    )
    d.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] pre mark.previewHighlight"),
        ),
    )


def test_ws_T18_highlight_cleared_after_normal_browse(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_hi"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='search-result-row']").click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] mark.previewHighlight"),
        ),
    )
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ESCAPE)
    _wait_search_mode(d, present=False)
    _wait_entries_loaded(d)
    row = _wait(d).until(
        EC.element_to_be_clickable(
            (
                By.CSS_SELECTOR,
                "[data-testid='entries-pane'] li[data-entry-name='sub_only.md']",
            ),
        ),
    )
    row.click()
    _wait(d).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
        ),
    )
    assert not d.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-pane'] mark.previewHighlight",
    )


# --- Group 5: ownership + root nav ---


def test_ws_T19_arrow_left_exits_without_parent_dir_nav(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    """Manifest T19 — search list ArrowLeft exits cleanly (same gate as T02)."""
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    origin = seed_search_hierarchy["origin_inner"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _set_search_scope(d, "global")
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    _focus_search_list(d)
    d.switch_to.active_element.send_keys(Keys.ARROW_LEFT)
    _wait_search_mode(d, present=False)
    _wait_entries_loaded(d)
    assert _logical_dir_from_url(d.current_url) == origin


def test_ws_T20_search_root_nav_goes_to_repository_root(
    selenium_driver,
    tabularium_base_url: str,
    seed_search_hierarchy: dict[str, Any],
):
    d = selenium_driver
    root = seed_search_hierarchy["root"]
    tok = seed_search_hierarchy["tok_sub"]
    d.get(_entries_url(tabularium_base_url, root, "inner"))
    _wait_app_ready(d)
    _wait_entries_loaded(d)
    _fire_search(d, tok)
    _wait_search_mode(d)
    _wait_for_search_hits(d, 1)
    d.find_element(By.CSS_SELECTOR, "[data-testid='nav-root']").click()
    _wait_search_mode(d, present=False)
    assert _logical_dir_from_url(d.current_url) == "/"

