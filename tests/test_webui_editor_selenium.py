"""Selenium coverage for meetings/webui.editor — Ferrum manifest T01–T14.

Prerequisites: TABULARIUM_TEST_URL pointing at http://10.90.1.122:<port> (AGENTS.md).
"""

from __future__ import annotations

import sys
import uuid
from pathlib import Path
from urllib.parse import quote

import pytest
import requests
from selenium.webdriver import Keys
from selenium.webdriver.common.by import By
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

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


def _api_doc_get_url(base: str, abs_path: str) -> str:
    trimmed = abs_path.strip("/")
    enc = "/".join(quote(s, safe="") for s in trimmed.split("/") if s)
    return f"{base.rstrip('/')}/api/doc/{enc}"


def _put_plain(base: str, rel_segments: str, body: str) -> None:
    r = requests.put(
        f"{base.rstrip('/')}/api/doc/{rel_segments}",
        data=body.encode("utf-8"),
        headers={"Content-Type": "text/plain; charset=utf-8"},
        timeout=20,
    )
    r.raise_for_status()


def _get_doc_json(base: str, abs_path: str) -> dict:
    r = requests.get(_api_doc_get_url(base, abs_path), timeout=20)
    r.raise_for_status()
    return r.json()


def _save_chord():
    return Keys.COMMAND + "s" if sys.platform == "darwin" else Keys.CONTROL + "s"


def _open_file_in_tree(driver, root_slug: str, file_name: str) -> None:
    driver.find_element(
        By.CSS_SELECTOR,
        f"[data-entry-name='{root_slug}']",
    ).click()
    _wait_entries_loaded(driver)
    _wait(driver).until(
        EC.presence_of_element_located(
            (
                By.CSS_SELECTOR,
                "[data-testid='entries-pane'] li[data-entry-kind='file']",
            ),
        ),
    )
    driver.find_element(
        By.CSS_SELECTOR,
        f"[data-entry-name='{file_name}']",
    ).click()


def _open_editor_doc(driver, base: str, root_slug: str, file_name: str) -> None:
    driver.get(base.rstrip("/") + "/entries")
    _wait_app_ready(driver)
    _wait_entries_loaded(driver)
    _open_file_in_tree(driver, root_slug, file_name)
    _wait(driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-edit']"),
        ),
    )


@pytest.fixture
def seed_editor_doc(tabularium_base_url: str) -> dict:
    slug = f"ed_{uuid.uuid4().hex[:10]}"
    name = "note.md"
    body = f"# Seed\n\nunique_seed_{uuid.uuid4().hex}\n"
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/{name}", body)
    return {"root": slug, "path": f"/{slug}/{name}", "content": body}


@pytest.fixture
def seed_editor_pair(tabularium_base_url: str) -> dict:
    slug = f"edpair_{uuid.uuid4().hex[:10]}"
    base = tabularium_base_url.rstrip("/")
    body_a = f"alpha_{uuid.uuid4().hex}"
    body_b = f"beta_{uuid.uuid4().hex}"
    _put_plain(base, f"{slug}/a.md", body_a)
    _put_plain(base, f"{slug}/b.md", body_b)
    return {
        "root": slug,
        "path_a": f"/{slug}/a.md",
        "path_b": f"/{slug}/b.md",
        "body_a": body_a,
        "body_b": body_b,
    }


# --- T01–T07 ---


def test_editor_t01_edit_absent_without_open_doc(
    selenium_driver,
    tabularium_base_url,
):
    """T01 — Edit button absent without open document."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url.rstrip("/") + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    assert not selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-edit']",
    )


def test_editor_t02_edit_present_when_doc_open(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T02 — Edit button present when document open."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    btn = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']")
    assert btn.is_displayed() and btn.is_enabled()


def test_editor_t03_click_edit_enters_mode(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T03 — Clicking Edit enters edit mode."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    assert selenium_driver.find_elements(By.CSS_SELECTOR, "[data-testid='preview-save']")
    assert selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-cancel-edit']",
    )
    assert not selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-edit']",
    )


def test_editor_t04_raw_toggle_disabled_in_edit_mode(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T04 — MD/RAW toggle disabled in edit mode."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    raw = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-testid='preview-raw-toggle']",
    )
    assert raw.get_attribute("disabled") is not None


def test_editor_t05_textarea_seeded(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T05 — Textarea seeded with document body."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    assert ta.get_attribute("value") == s["content"]


def test_editor_t06_save_disabled_when_unchanged(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T06 — Save disabled when content unchanged."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    save = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-save']")
    assert save.get_attribute("disabled") is not None


def test_editor_t07_save_enabled_after_change(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T07 — Save enabled after content change."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    ta.clear()
    token = f"edited_{uuid.uuid4().hex}"
    ta.send_keys(token)
    _wait(selenium_driver).until(
        lambda d: d.find_element(
            By.CSS_SELECTOR,
            "[data-testid='preview-save']",
        ).get_attribute("disabled")
        is None,
    )


# --- T08–T11 ---


def test_editor_t08_cancel_exits_without_save(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T08 — Cancel exits edit mode without saving."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    token = f"ghost_{uuid.uuid4().hex}"
    ta.clear()
    ta.send_keys(token)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-testid='preview-cancel-edit']",
    ).click()
    _wait(selenium_driver).until(
        EC.invisibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-edit']"),
        ),
    )
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
    assert token not in pane.text
    remote = _get_doc_json(base, s["path"])["content"]
    assert remote == s["content"]
    assert token not in remote


def test_editor_t09_successful_save_refreshes(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T09 — Successful save exits edit mode and refreshes preview."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    token = f"saved_{uuid.uuid4().hex}"
    ta.clear()
    ta.send_keys(token)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-save']").click()
    _wait(selenium_driver).until(
        EC.invisibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-edit']"),
        ),
    )
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
    assert token in pane.text
    remote = _get_doc_json(base, s["path"])["content"]
    assert token in remote


def test_editor_t10_ctrl_s_saves_when_dirty(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T10 — Ctrl/Cmd+S saves when dirty."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    token = f"keys_{uuid.uuid4().hex}"
    ta.clear()
    ta.send_keys(token)
    ta.send_keys(_save_chord())
    _wait(selenium_driver).until(
        EC.invisibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    remote = _get_doc_json(base, s["path"])["content"]
    assert token in remote


def test_editor_t11_save_failure_inline_error(
    selenium_driver,
    tabularium_base_url,
    seed_editor_doc: dict,
):
    """T11 — Save failure shows inline error; stays in edit mode.

    Backend PUT is an upsert, so DELETE-then-save recreates the file and does not
    fail. We stub `window.fetch` to return an HTTP 500 for document PUTs so the
    UI exercises the normal `putDocument` error path.
    """
    selenium_driver.set_window_size(1200, 800)
    s = seed_editor_doc
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, s["root"], "note.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    ta.send_keys(" x")
    selenium_driver.execute_script(
        """
        window.__tabulariumOrigFetch = window.fetch;
        window.fetch = function(input, init) {
          const url = typeof input === 'string' ? input : (input && input.url) || '';
          const m = (init && init.method) || 'GET';
          if (String(m).toUpperCase() === 'PUT' && url.indexOf('/api/doc') !== -1) {
            return Promise.resolve(
              new Response('T11 simulated save failure', {
                status: 500,
                statusText: 'Internal Server Error',
                headers: { 'Content-Type': 'text/plain; charset=utf-8' },
              }),
            );
          }
          return window.__tabulariumOrigFetch.call(this, input, init);
        };
        """
    )
    try:
        selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-save']").click()
        _wait(selenium_driver).until(
            EC.visibility_of_element_located(
                (By.CSS_SELECTOR, "[data-testid='preview-save-error']"),
            ),
        )
        err_el = selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-testid='preview-save-error']",
        )
        assert "T11 simulated save failure" in err_el.text
        assert selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-testid='preview-editor']",
        ).is_displayed()
        assert _get_doc_json(base, s["path"])["content"] == s["content"]
    finally:
        selenium_driver.execute_script(
            """
            if (window.__tabulariumOrigFetch) {
              window.fetch = window.__tabulariumOrigFetch;
              delete window.__tabulariumOrigFetch;
            }
            """
        )


# --- T12–T14 ---


def test_editor_t12_dirty_nav_cancel_keeps_editor(
    selenium_driver,
    tabularium_base_url,
    seed_editor_pair: dict,
):
    """T12 — Dirty navigation: dismiss confirm keeps editor."""
    selenium_driver.set_window_size(1200, 800)
    p = seed_editor_pair
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, p["root"], "a.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    token = f"dirty_{uuid.uuid4().hex}"
    ta.send_keys(token)
    selenium_driver.execute_script("window.confirm = function() { return false; };")
    try:
        selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-entry-name='b.md']",
        ).click()

        def _still_dirty_same_draft(driver):
            try:
                el = driver.find_element(
                    By.CSS_SELECTOR,
                    "[data-testid='preview-editor']",
                )
                if not el.is_displayed():
                    return False
                val = el.get_attribute("value") or ""
                return token in val
            except Exception:
                return False

        _wait(selenium_driver).until(_still_dirty_same_draft)
    finally:
        selenium_driver.execute_script(
            "window.confirm = function() { return true; };",
        )


def test_editor_t13_dirty_nav_accept_leaves(
    selenium_driver,
    tabularium_base_url,
    seed_editor_pair: dict,
):
    """T13 — Dirty navigation: accept confirm loads other file."""
    selenium_driver.set_window_size(1200, 800)
    p = seed_editor_pair
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, p["root"], "a.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    ta.send_keys("noise")
    selenium_driver.execute_script("window.confirm = function() { return true; };")
    try:
        selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-entry-name='b.md']",
        ).click()
        _wait(selenium_driver).until(
            EC.invisibility_of_element_located(
                (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
            ),
        )
        pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
        assert p["body_b"] in pane.text
    finally:
        selenium_driver.execute_script(
            "window.confirm = function() { return true; };",
        )


def test_editor_t14_same_file_skips_confirm(
    selenium_driver,
    tabularium_base_url,
    seed_editor_pair: dict,
):
    """T14 — Re-selecting same file does not call confirm."""
    selenium_driver.set_window_size(1200, 800)
    p = seed_editor_pair
    base = tabularium_base_url.rstrip("/")
    _open_editor_doc(selenium_driver, base, p["root"], "a.md")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    ta = _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-editor']"),
        ),
    )
    ta.send_keys("mut")
    selenium_driver.execute_script(
        "window.__confirmFired = false;"
        "window.confirm = function() { window.__confirmFired = true; return true; };",
    )
    try:
        selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-entry-name='a.md']",
        ).click()

        def _no_confirm_and_editor_visible(driver):
            if driver.execute_script("return window.__confirmFired;"):
                return False
            try:
                el = driver.find_element(
                    By.CSS_SELECTOR,
                    "[data-testid='preview-editor']",
                )
                return el.is_displayed()
            except Exception:
                return False

        _wait(selenium_driver).until(_no_confirm_and_editor_visible)
    finally:
        selenium_driver.execute_script(
            "window.confirm = function() { return true; };",
        )


GFM_TABLE_FIXTURE = Path(__file__).resolve().parent / "fixtures" / "gfm_table_fixture.md"


@pytest.fixture
def seed_gfm_table_preview_doc(tabularium_base_url: str) -> dict:
    slug = f"gfm_ed_{uuid.uuid4().hex[:10]}"
    name = "tables.md"
    body = GFM_TABLE_FIXTURE.read_text(encoding="utf-8")
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/{name}", body)
    return {"root": slug, "name": name}


def test_editor_gfm_preview_renders_table(
    selenium_driver,
    tabularium_base_url,
    seed_gfm_table_preview_doc: dict,
):
    """Preview pane renders GFM pipe tables as HTML (`meetings/tables`)."""
    selenium_driver.set_window_size(1200, 800)
    s = seed_gfm_table_preview_doc
    _open_editor_doc(selenium_driver, tabularium_base_url, s["root"], s["name"])
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
    tables = pane.find_elements(By.CSS_SELECTOR, "table")
    assert len(tables) >= 1
    assert tables[0].find_elements(By.TAG_NAME, "th")
    assert tables[0].find_elements(By.TAG_NAME, "td")
