"""Selenium coverage for meetings/webui.chat — Ferrum manifest (chat surface)."""

from __future__ import annotations

import uuid

import pytest
import requests
from selenium.common.exceptions import StaleElementReferenceException
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


def _put_plain(base: str, rel_segments: str, body: str) -> None:
    r = requests.put(
        f"{base.rstrip('/')}/api/doc/{rel_segments}",
        data=body.encode("utf-8"),
        headers={"Content-Type": "text/plain; charset=utf-8"},
        timeout=20,
    )
    r.raise_for_status()


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


@pytest.fixture
def seed_chat_doc(tabularium_base_url: str) -> dict:
    slug = f"ch_{uuid.uuid4().hex[:10]}"
    name = "room.md"
    body = f"# Chat seed\n\nhello_{uuid.uuid4().hex}\n"
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/{name}", body)
    return {"root": slug, "path": f"/{slug}/{name}", "content": body}


def _preview_pane_contains(driver, needle: str) -> bool:
    try:
        return needle in driver.find_element(
            By.CSS_SELECTOR,
            "[data-testid='preview-pane']",
        ).text
    except StaleElementReferenceException:
        return False


def _wait_preview_pane_contains(driver, needle: str, timeout: float = 25) -> None:
    _wait(driver, timeout=timeout).until(lambda d: _preview_pane_contains(d, needle))


def _wait_preview_chat_visible(driver, timeout: float = 25) -> None:
    _wait(driver, timeout=timeout).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-chat']")),
    )


def _wait_chat_connected(selenium_driver, needle: str) -> None:
    _wait(selenium_driver, timeout=25).until(
        lambda d: needle
        in d.find_element(
            By.CSS_SELECTOR,
            "[data-testid='chat-transcript']",
        ).text,
    )


def test_chat_c01_button_absent_without_open_doc(
    selenium_driver,
    tabularium_base_url,
):
    """C01 — Chat toggle absent without an open document."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.get(tabularium_base_url.rstrip("/") + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    assert not selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-chat']",
    )


def test_chat_c02_gate_start_and_transcript(
    selenium_driver,
    tabularium_base_url,
    seed_chat_doc: dict,
):
    """C02 — No cookie: gate → Start → transcript + composer; body text visible."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    base = tabularium_base_url.rstrip("/")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, seed_chat_doc["root"], "room.md")
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-gate']")),
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']").send_keys(
        "selenium_ferrum",
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-start']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-composer']")),
    )
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-transcript']")),
    )
    # Transcript is rendered markdown — heading text omits leading "#".
    _wait_chat_connected(selenium_driver, "Chat seed")


def test_chat_c03_save_from_editor_returns_to_chat_surface(
    selenium_driver,
    tabularium_base_url,
    seed_chat_doc: dict,
):
    """C03 — From chat, full editor Save returns to chat (not plain preview)."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    base = tabularium_base_url.rstrip("/")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, seed_chat_doc["root"], "room.md")
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']").send_keys(
        "scribe",
    )
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-start']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-composer']")),
    )
    _wait_chat_connected(selenium_driver, "Chat seed")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-editor']")),
    )
    editor = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-editor']")
    editor.send_keys("\n# after save\n")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-save']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-composer']")),
    )
    assert not selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-editor']",
    )


def test_chat_c04_send_disabled_while_empty(
    selenium_driver,
    tabularium_base_url,
    seed_chat_doc: dict,
):
    """C04 — Send disabled for whitespace-only input."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    base = tabularium_base_url.rstrip("/")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, seed_chat_doc["root"], "room.md")
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']").send_keys("u1")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-start']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-composer']")),
    )
    _wait_chat_connected(selenium_driver, "Chat seed")
    send = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-send']")
    assert send.get_attribute("disabled")
    comp = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-composer']")
    comp.send_keys("   ")
    assert send.get_attribute("disabled")


def test_chat_c05_edit_loads_fresh_body_after_remote_write(
    selenium_driver,
    tabularium_base_url,
):
    """C05 — Remote PUT after preview load; Edit opens server body, not stale preview."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    slug = f"ch5_{uuid.uuid4().hex[:10]}"
    m_old = f"STALE_{uuid.uuid4().hex}"
    m_new = f"FRESH_{uuid.uuid4().hex}"
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/note.md", f"# t\n\n{m_old}\n")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, slug, "note.md")
    _wait_preview_pane_contains(selenium_driver, m_old)
    _put_plain(base, f"{slug}/note.md", f"# t\n\n{m_new}\n")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-edit']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='preview-editor']")),
    )
    editor = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-editor']")
    val = editor.get_attribute("value") or ""
    assert m_new in val
    assert m_old not in val


def test_chat_c06_preview_after_chat_triggers_reload(
    selenium_driver,
    tabularium_base_url,
):
    """C06 — Chat → Preview bumps reload; server-updated body visible in preview."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    slug = f"ch6_{uuid.uuid4().hex[:10]}"
    m0 = f"INIT_{uuid.uuid4().hex}"
    m1 = f"POSTCHAT_{uuid.uuid4().hex}"
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/live.md", f"# c\n\n{m0}\n")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, slug, "live.md")
    _wait_preview_pane_contains(selenium_driver, m0)
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']").send_keys("c06")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-start']").click()
    _wait_chat_connected(selenium_driver, m0)
    _put_plain(base, f"{slug}/live.md", f"# c\n\n{m1}\n")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    _wait_preview_pane_contains(selenium_driver, m1, timeout=25)
    assert not _preview_pane_contains(selenium_driver, m0)


def test_chat_c07_chat_opens_focus_on_gate_input(
    selenium_driver,
    tabularium_base_url,
    seed_chat_doc: dict,
):
    """C07 — No cookie: Chat focuses nickname gate input."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    base = tabularium_base_url.rstrip("/")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, seed_chat_doc["root"], "room.md")
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-gate-input']")),
    )
    gate = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']")
    active = selenium_driver.switch_to.active_element
    assert active.get_attribute("data-testid") == gate.get_attribute("data-testid")


def test_chat_c08_start_focuses_composer(
    selenium_driver,
    tabularium_base_url,
    seed_chat_doc: dict,
):
    """C08 — After Start, composer textarea is focused."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    base = tabularium_base_url.rstrip("/")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, seed_chat_doc["root"], "room.md")
    _wait_preview_chat_visible(selenium_driver)
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-chat']").click()
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-gate-input']").send_keys("c08")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-start']").click()
    _wait(selenium_driver).until(
        EC.visibility_of_element_located((By.CSS_SELECTOR, "[data-testid='chat-composer']")),
    )
    comp = selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='chat-composer']")
    active = selenium_driver.switch_to.active_element
    assert active.get_attribute("data-testid") == comp.get_attribute("data-testid")


def test_chat_c09_raw_toggle_triggers_reload(
    selenium_driver,
    tabularium_base_url,
):
    """C09 — RAW/MD toggle bumps reload; updated server body visible."""
    selenium_driver.set_window_size(1200, 800)
    selenium_driver.delete_all_cookies()
    slug = f"ch9_{uuid.uuid4().hex[:10]}"
    m0 = f"V0_{uuid.uuid4().hex}"
    m1 = f"V1_{uuid.uuid4().hex}"
    base = tabularium_base_url.rstrip("/")
    _put_plain(base, f"{slug}/raw.md", f"# r\n\n{m0}\n")
    selenium_driver.get(base + "/entries")
    _wait_app_ready(selenium_driver)
    _wait_entries_loaded(selenium_driver)
    _open_file_in_tree(selenium_driver, slug, "raw.md")
    _wait_preview_pane_contains(selenium_driver, m0)
    _put_plain(base, f"{slug}/raw.md", f"# r\n\n{m1}\n")
    selenium_driver.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    _wait_preview_pane_contains(selenium_driver, m1, timeout=25)
    assert not _preview_pane_contains(selenium_driver, m0)
