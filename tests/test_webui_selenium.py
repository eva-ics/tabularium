"""Selenium rites for embedded Web UI — see meetings/webui.v1 (Ferrum checklist)."""

from __future__ import annotations

import re
import time
import uuid
from typing import Any

import pytest
import requests
from selenium.webdriver import Keys
from selenium.webdriver.common.by import By
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

from tests.helpers import mkdir

pytestmark = pytest.mark.webui


def _rpc(base: str, method: str, params: dict[str, Any]) -> dict:
    r = requests.post(
        f"{base.rstrip('/')}/rpc",
        json={"jsonrpc": "2.0", "method": method, "params": params, "id": 1},
        timeout=20,
    )
    r.raise_for_status()
    return r.json()


def _mkdir_path(base: str, path: str) -> None:
    p = path if path.startswith("/") else f"/{path}"
    requests.post(
        f"{base.rstrip('/')}/api/doc",
        json={"path": p, "description": None},
        timeout=20,
    ).raise_for_status()


def _preview_scroll_body(driver):
    return driver.find_element(
        By.CSS_SELECTOR,
        "[data-testid='preview-pane'] [tabindex='-1']",
    )


def _entry_names_in_dom_order(driver) -> list[str]:
    # Single DOM read — avoids stale element references after React reorders on sort.
    return driver.execute_script(
        "return Array.from(document.querySelectorAll("
        "'[data-testid=\"entries-pane\"] li[role=\"option\"][data-entry-name]'))"
        ".map(el => el.getAttribute('data-entry-name') || '');"
    )


def _selected_has_nav(driver, nav: str) -> bool:
    els = driver.find_elements(
        By.CSS_SELECTOR,
        f"[data-testid='entries-pane'] li[data-nav='{nav}'][data-selected='true']",
    )
    return len(els) > 0


def _selected_entry_name(driver) -> str | None:
    els = driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='entries-pane'] li[data-selected='true'][data-entry-name]",
    )
    if not els:
        return None
    return els[0].get_attribute("data-entry-name")


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
        EC.invisibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-loading']")),
    )


def _open_seed_dir_and_wait(driver, seed_dir: str):
    driver.find_element(
        By.CSS_SELECTOR,
        f"[data-entry-name='{seed_dir}']",
    ).click()
    _wait_entries_loaded(driver)
    # Overlay can clear a frame before entry rows commit (Chrome / React timing).
    _wait(driver).until(
        EC.presence_of_element_located(
            (
                By.CSS_SELECTOR,
                "[data-testid='entries-pane'] li[data-entry-kind='file']",
            ),
        ),
    )


@pytest.fixture
def seed_dir(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"selenium_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    for fname, body in (
        ("readme.md", "# Title\n\n**Bold** machine spirit."),
        ("plain.txt", "no markdown here"),
    ):
        requests.post(
            f"{base}/api/doc/{name}",
            json={"name": fname, "content": body},
            timeout=15,
        ).raise_for_status()
    return name


@pytest.fixture
def seed_sort_dir(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"sort_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    _mkdir_path(base, f"/{name}/inner")
    for fname, body in (
        ("zzz.md", "z"),
        ("aaa.md", "a"),
        ("mmm.md", "m"),
    ):
        requests.post(
            f"{base}/api/doc/{name}",
            json={"name": fname, "content": body},
            timeout=15,
        ).raise_for_status()
    return name


@pytest.fixture
def seed_size_dir(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"size_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "tiny.bin", "content": "x"},
        timeout=15,
    ).raise_for_status()
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "fat.bin", "content": "y" * 8000},
        timeout=15,
    ).raise_for_status()
    return name


@pytest.fixture
def seed_mtime_dir(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"mtime_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "old.txt", "content": "old"},
        timeout=15,
    ).raise_for_status()
    time.sleep(1.2)
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "new.txt", "content": "new"},
        timeout=15,
    ).raise_for_status()
    return name


@pytest.fixture
def seed_long_doc(tabularium_base_url: str, seed_dir: str) -> str:
    base = tabularium_base_url.rstrip("/")
    body = "\n\n".join(f"Line {i} scroll corpus sanctified." for i in range(120))
    requests.post(
        f"{base}/api/doc/{seed_dir}",
        json={"name": "longscroll.md", "content": body},
        timeout=20,
    ).raise_for_status()
    return seed_dir


@pytest.fixture
def seed_nested(tabularium_base_url: str, seed_dir: str) -> str:
    base = tabularium_base_url.rstrip("/")
    _mkdir_path(base, f"/{seed_dir}/nested")
    return seed_dir


@pytest.fixture
def seed_big_file(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"big_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    chunk = "x" * (512 * 1024)
    big = chunk * 3
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "chunky.bin", "content": big},
        timeout=60,
    ).raise_for_status()
    return name


def test_app_loads_entries_visible(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-pane']")),
    )


def test_directory_navigation_click(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )


def test_directory_navigation_keyboard(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    for _ in range(200):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            f"[data-selected='true'][data-entry-name='{seed_dir}']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail("keyboard could not select seeded directory row")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )
    _wait_entries_loaded(selenium_driver)


def test_file_opens_preview(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )


def test_escape_clears_preview_desktop(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Select a file",
        ),
    )


def test_left_right_switch_panes(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ARROW_RIGHT)
    body.send_keys(Keys.ARROW_LEFT)


def test_mobile_back_button(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(375, 812)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='mobile-back']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='mobile-back']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-pane']")),
    )


def test_entries_stats_switch(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='top-nav-stats']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='stats-root']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='top-nav-entries']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-pane']")),
    )


def test_spa_deep_link_not_404(selenium_driver, tabularium_base_url):
    selenium_driver.get(
        tabularium_base_url + "/entries/library/deep/scroll/exemplar",
    )
    _wait_app_ready(selenium_driver)
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-pane']")),
    )
    assert "404" not in selenium_driver.title.lower()


# --- Group E — infrastructure ---


def test_e1_rpc_returns_json_not_html(tabularium_base_url: str):
    r = requests.post(
        f"{tabularium_base_url.rstrip('/')}/rpc",
        json={
            "jsonrpc": "2.0",
            "method": "list_directory",
            "params": {"path": "/"},
            "id": 1,
        },
        timeout=15,
    )
    r.raise_for_status()
    ct = r.headers.get("Content-Type", "")
    assert "application/json" in ct
    assert not r.text.lstrip().upper().startswith("<!DOCTYPE")


# --- Group A — navigation & structure ---


def test_a1_root_nav_goes_to_slash(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-root']").click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            "/",
        ),
    )


def test_a2_parent_nav_present_only_in_subdir(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    assert not selenium_driver.find_elements(By.CSS_SELECTOR, "[data-testid='nav-parent']")
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='nav-parent']")),
    )


def test_a3_parent_nav_click(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            "/",
        ),
    )


def test_a4_parent_auto_selected_on_enter_subdir(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='nav-parent'][data-selected='true']"),
        ),
    )


def test_a5_url_reflects_directory_navigation(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    _wait(selenium_driver).until(
        lambda d: seed_dir in d.current_url and "/entries/" in d.current_url,
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait(selenium_driver).until(
        lambda d: "/entries" in d.current_url
        and f"/{seed_dir}" not in d.current_url.replace("/entries/", ""),
    )


def test_a6_url_reflects_open_query(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(lambda d: "open=" in d.current_url)


# --- Group B — entry display ---


def test_b1_modification_date_visible(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    row = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    )
    assert re.search(r"20[0-9]{2}|Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec", row.text)


def test_b2_description_subline(selenium_driver, tabularium_base_url, seed_dir: str):
    base = tabularium_base_url.rstrip("/")
    note = "Selenium description doctrina"
    _rpc(
        base,
        "describe",
        {"path": f"/{seed_dir}/readme.md", "description": note},
    )
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    row = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    )
    assert note in row.text


def test_b3_large_file_size_not_raw_seven_digits(selenium_driver, tabularium_base_url, seed_big_file: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_big_file)
    row = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='chunky.bin']",
    )
    assert not re.search(r"\d{7,}", row.text)


def test_b4_dir_file_glyphs(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='entries-pane']")
    dir_row = pane.find_element(
        By.XPATH,
        f".//li[@role='option'][@data-entry-name='{seed_dir}']",
    )
    assert dir_row.get_attribute("data-entry-kind") == "dir"
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    file_row = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    )
    assert file_row.get_attribute("data-entry-kind") == "file"


# --- Group C — preview ---


def test_c1_markdown_renders_html_not_raw_markers(
    selenium_driver, tabularium_base_url, seed_dir: str
):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] strong"),
        ),
    )
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
    assert "**Bold**" not in pane.text


def test_c2_preview_arrow_keys_scroll(selenium_driver, tabularium_base_url, seed_long_doc: str):
    selenium_driver.set_window_size(900, 500)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    _select_seed_dir_keyboard(selenium_driver, seed_long_doc)
    body.send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    for _ in range(80):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='longscroll.md']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail("could not select longscroll.md")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Line 50",
        ),
    )
    scroll_el = _preview_scroll_body(selenium_driver)
    t0 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    body.send_keys(Keys.ARROW_DOWN)
    body.send_keys(Keys.ARROW_DOWN)
    body.send_keys(Keys.ARROW_DOWN)
    t1 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t1 > t0
    body.send_keys(Keys.ARROW_UP)
    t2 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t2 < t1


def test_c3_preview_page_up_down_scroll(selenium_driver, tabularium_base_url, seed_long_doc: str):
    selenium_driver.set_window_size(900, 500)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    _select_seed_dir_keyboard(selenium_driver, seed_long_doc)
    body.send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    for _ in range(80):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='longscroll.md']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail("could not select longscroll.md")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Line 50",
        ),
    )
    scroll_el = _preview_scroll_body(selenium_driver)
    t0 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    body.send_keys(Keys.PAGE_DOWN)
    t1 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t1 > t0
    body.send_keys(Keys.PAGE_UP)
    t2 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t2 < t1


# --- Group D — stats ---


def test_d1_stats_shows_document_count(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/stats")
    _wait_app_ready(selenium_driver)
    card = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='stats-total-files']")
    _wait(selenium_driver).until(lambda d: re.search(r"[0-9]+", card.text))


def test_d2_stats_shows_size_mb(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/stats")
    _wait_app_ready(selenium_driver)
    mb = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='stats-total-mb']")
    _wait(selenium_driver).until(lambda d: re.search(r"mb", mb.text, re.I))
    assert not re.search(r"\b[0-9]{7,}\s*B\b", mb.text, re.I)


def test_d3_stats_canvas_present(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/stats")
    _wait_app_ready(selenium_driver)
    _wait(selenium_driver).until(
        EC.visibility_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='stats-root'] canvas"),
        ),
    )


# --- Group F — sort headers (v1a) ---


def test_f1_sort_headers_visible(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    for tid in ("sort-name", "sort-size", "sort-modified"):
        _wait(selenium_driver).until(
            EC.visibility_of_element_located((By.CSS_SELECTOR, f"[data-testid='{tid}']")),
        )


def test_f2_name_sort_toggles_order(
    selenium_driver, tabularium_base_url, seed_sort_dir: str
):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_sort_dir)
    names0 = _entry_names_in_dom_order(selenium_driver)
    assert set(names0) >= {"aaa.md", "mmm.md", "zzz.md"}
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    time.sleep(0.15)
    asc = _entry_names_in_dom_order(selenium_driver)
    only = [n for n in asc if n.endswith(".md")]
    assert only == sorted(only)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    time.sleep(0.15)
    desc = _entry_names_in_dom_order(selenium_driver)
    only_d = [n for n in desc if n.endswith(".md")]
    assert only_d == sorted(only_d, reverse=True)


def test_f3_sort_header_arrow_flips(selenium_driver, tabularium_base_url, seed_sort_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_sort_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    time.sleep(0.15)
    t1 = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").text
    assert "↑" in t1 or "↓" in t1
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    time.sleep(0.15)
    t2 = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").text
    assert t1 != t2


def test_f4_size_sort_orders_files(selenium_driver, tabularium_base_url, seed_size_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_size_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-size']").click()
    time.sleep(0.15)
    names = _entry_names_in_dom_order(selenium_driver)
    bins = [n for n in names if n.endswith(".bin")]
    assert bins.index("tiny.bin") < bins.index("fat.bin")


def test_f5_modified_sort_orders_newest_last_on_asc_click(
    selenium_driver, tabularium_base_url, seed_mtime_dir: str
):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_mtime_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-modified']").click()
    time.sleep(0.15)
    names = _entry_names_in_dom_order(selenium_driver)
    txts = [n for n in names if n.endswith(".txt")]
    assert txts.index("old.txt") < txts.index("new.txt")


def test_f6_sort_persists_across_directory_navigation(
    selenium_driver, tabularium_base_url, seed_sort_dir: str
):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_sort_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']").click()
    time.sleep(0.15)
    desc_before = _entry_names_in_dom_order(selenium_driver)
    only_b = [n for n in desc_before if n.endswith(".md")]
    assert only_b == sorted(only_b, reverse=True)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='inner']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            "/inner",
        ),
    )
    _wait_entries_loaded(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_sort_dir}",
        ),
    )
    _wait_entries_loaded(selenium_driver)
    btn = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='sort-name']")
    assert "↑" in btn.text or "↓" in btn.text
    desc_after = _entry_names_in_dom_order(selenium_driver)
    only_a = [n for n in desc_after if n.endswith(".md")]
    assert only_a == sorted(only_a, reverse=True)


def test_f7_no_sort_arrow_on_fresh_load(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    for tid in ("sort-name", "sort-size", "sort-modified"):
        t = selenium_driver.find_element(By.CSS_SELECTOR, f"[data-testid='{tid}']").text
        assert "↑" not in t and "↓" not in t


# --- Group G — RAW / MD ---


def test_g1_raw_toggle_absent_without_doc(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    assert not selenium_driver.find_elements(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']")


def test_g2_raw_toggle_visible_when_doc_open(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']")),
    )
    assert (
        selenium_driver.find_element(
            By.CSS_SELECTOR,
            "[data-testid='preview-raw-toggle']",
        ).text
        == "RAW"
    )


def test_g3_raw_toggle_shows_plain_markers(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] strong"),
        ),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']"),
            "MD",
        ),
    )
    assert not selenium_driver.find_elements(By.CSS_SELECTOR, "[data-testid='preview-pane'] strong")
    pane = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane']")
    assert "**Bold**" in pane.text


def test_g4_md_toggle_restores_render(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    _wait(selenium_driver).until(
        EC.element_to_be_clickable((By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] strong"),
        ),
    )


def test_g5_raw_resets_on_new_file(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='plain.txt']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']"),
            "RAW",
        ),
    )


# --- Group H — keyboard navigation ---


def _select_seed_dir_keyboard(driver, seed_dir: str):
    _wait_entries_loaded(driver)
    body = driver.find_element(By.TAG_NAME, "body")
    for _ in range(200):
        body.send_keys(Keys.ARROW_DOWN)
        if driver.find_elements(
            By.CSS_SELECTOR,
            f"[data-selected='true'][data-entry-name='{seed_dir}']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail(f"keyboard could not select {seed_dir}")


def test_h1_arrow_down_moves_selection(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    assert _selected_has_nav(selenium_driver, "root")
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ARROW_DOWN)
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-selected='true'][data-entry-name]"),
        ),
    )


def test_h2_arrow_up_stops_at_zero(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ARROW_DOWN)
    _wait(selenium_driver).until(
        lambda d: _selected_entry_name(d) is not None
        or _selected_has_nav(d, "parent")
        or not _selected_has_nav(d, "root"),
    )
    body.send_keys(Keys.ARROW_UP)
    assert _selected_has_nav(selenium_driver, "root")
    body.send_keys(Keys.ARROW_UP)
    assert _selected_has_nav(selenium_driver, "root")


def test_h3_arrow_down_clamped_to_last(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )
    _wait_entries_loaded(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    for _ in range(500):
        body.send_keys(Keys.ARROW_DOWN)
    names = _entry_names_in_dom_order(selenium_driver)
    assert _selected_entry_name(selenium_driver) == names[-1]


def test_h4_enter_opens_subdirectory(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )
    _wait_entries_loaded(selenium_driver)


def test_h5_enter_opens_file_preview(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    for _ in range(50):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='readme.md']",
        ):
            break
    else:
        pytest.fail("readme not selected")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )


def test_h6_arrow_right_enters_directory(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ARROW_RIGHT)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            f"/{seed_dir}",
        ),
    )
    _wait_entries_loaded(selenium_driver)


def test_h7_arrow_right_opens_file(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ARROW_RIGHT)
    _wait_entries_loaded(selenium_driver)
    for _ in range(50):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='readme.md']",
        ):
            break
    body.send_keys(Keys.ARROW_RIGHT)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )


def test_h8_left_from_preview_focuses_tree(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ARROW_RIGHT)
    _wait_entries_loaded(selenium_driver)
    for _ in range(50):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='readme.md']",
        ):
            break
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )
    body.send_keys(Keys.ARROW_LEFT)
    body.send_keys(Keys.ARROW_DOWN)
    names = _entry_names_in_dom_order(selenium_driver)
    assert _selected_entry_name(selenium_driver) in names


def test_h9_left_goes_to_parent(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _select_seed_dir_keyboard(selenium_driver, seed_dir)
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ARROW_LEFT)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            "/",
        ),
    )


def test_h10_left_at_root_selects_first_entry(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    assert _selected_has_nav(selenium_driver, "root")
    body.send_keys(Keys.ARROW_LEFT)
    first = _entry_names_in_dom_order(selenium_driver)[0]
    _wait(selenium_driver).until(
        lambda d: _selected_entry_name(d) == first,
    )
    for _ in range(30):
        body.send_keys(Keys.ARROW_DOWN)
    body.send_keys(Keys.ARROW_LEFT)
    _wait(selenium_driver).until(
        lambda d: _selected_entry_name(d) == first,
    )


def test_h11_left_at_root_empty_list_noop(selenium_driver, tabularium_base_url):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    if _entry_names_in_dom_order(selenium_driver):
        pytest.skip("repository root has entries — cannot assert empty-root left no-op")
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    assert _selected_has_nav(selenium_driver, "root")
    body.send_keys(Keys.ARROW_LEFT)
    assert _selected_has_nav(selenium_driver, "root")


def test_h12_preview_focus_arrow_scrolls_not_list(
    selenium_driver, tabularium_base_url, seed_long_doc: str
):
    selenium_driver.set_window_size(900, 500)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    _select_seed_dir_keyboard(selenium_driver, seed_long_doc)
    body.send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    for _ in range(80):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='longscroll.md']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail("could not select longscroll.md")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Line 50",
        ),
    )
    scroll_el = _preview_scroll_body(selenium_driver)
    sel_before = _selected_entry_name(selenium_driver)
    t0 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    body.send_keys(Keys.ARROW_DOWN)
    t1 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t1 > t0
    assert _selected_entry_name(selenium_driver) == sel_before


def test_h13_preview_page_keys(selenium_driver, tabularium_base_url, seed_long_doc: str):
    selenium_driver.set_window_size(900, 500)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    _select_seed_dir_keyboard(selenium_driver, seed_long_doc)
    body.send_keys(Keys.ENTER)
    _wait_entries_loaded(selenium_driver)
    for _ in range(80):
        body.send_keys(Keys.ARROW_DOWN)
        if selenium_driver.find_elements(
            By.CSS_SELECTOR,
            "[data-selected='true'][data-entry-name='longscroll.md']",
        ):
            break
        time.sleep(0.02)
    else:
        pytest.fail("could not select longscroll.md")
    body.send_keys(Keys.ENTER)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Line 50",
        ),
    )
    scroll_el = _preview_scroll_body(selenium_driver)
    body.send_keys(Keys.PAGE_DOWN)
    t1 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t1 > 0
    body.send_keys(Keys.PAGE_UP)
    t2 = selenium_driver.execute_script("return arguments[0].scrollTop;", scroll_el)
    assert t2 < t1


def test_h14_escape_clears_and_tree_focus_moves_selection(
    selenium_driver, tabularium_base_url, seed_dir: str
):
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Bold",
        ),
    )
    body = selenium_driver.find_element(By.TAG_NAME, "body")
    body.send_keys(Keys.ESCAPE)
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Select a file",
        ),
    )
    _wait(selenium_driver).until(lambda d: _selected_entry_name(d) == "readme.md")
    body.send_keys(Keys.ARROW_UP)
    _wait(selenium_driver).until(lambda d: _selected_entry_name(d) == "plain.txt")


def test_h15_escape_mobile_returns_to_entries(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.set_window_size(375, 812)
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='readme.md']",
    ).click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='mobile-back']")),
    )
    selenium_driver.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='entries-pane']")),
    )


def test_h16_back_navigation_restores_selection(selenium_driver, tabularium_base_url, seed_dir: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_dir)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-parent']").click()
    _wait_entries_loaded(selenium_driver)
    _wait(selenium_driver).until(
        lambda d: _selected_entry_name(d) == seed_dir,
    )


def test_h17_root_click_no_restore_memory(selenium_driver, tabularium_base_url, seed_nested: str):
    selenium_driver.get(tabularium_base_url + "/entries")
    _wait_app_ready(selenium_driver)
    _open_seed_dir_and_wait(selenium_driver, seed_nested)
    selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-entry-name='nested']",
    ).click()
    _wait_entries_loaded(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='nav-root']").click()
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='entries-path']"),
            "/",
        ),
    )
    assert _selected_has_nav(selenium_driver, "root")
