"""Selenium rites for internal markdown link refs — Ferrum QA manifest.

Covers the heresy fixed in PreviewPane: relative/absolute internal refs must
resolve to proper /entries/<dir>?open=<abs_path> URLs so that both click
navigation and copy-link work correctly.

Prerequisites: TABULARIUM_TEST_URL pointing at http://10.90.1.102:<port> (AGENTS.md).
"""

from __future__ import annotations

import uuid
from urllib.parse import quote, urlencode

import pytest
import requests
from selenium.webdriver.common.by import By
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

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


def _open_dir_and_wait(driver, base_url: str, dir_name: str) -> None:
    driver.get(f"{base_url}/entries")
    _wait_app_ready(driver)
    driver.find_element(By.CSS_SELECTOR, f"[data-entry-name='{dir_name}']").click()
    _wait_entries_loaded(driver)
    _wait(driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='entries-pane'] li[data-entry-kind='file']"),
        ),
    )


def _open_doc_url(base_url: str, dir_name: str, doc_name: str) -> str:
    """Build the direct URL for opening a file in preview."""
    open_val = f"/{dir_name}/{doc_name}"
    return f"{base_url}/entries/{quote(dir_name, safe='')}?{urlencode({'open': open_val})}"


@pytest.fixture
def ref_dir(tabularium_base_url: str) -> str:
    """Directory with index.md (relative + external + hash links) and target.md."""
    slug = uuid.uuid4().hex[:8]
    name = f"refs_{slug}"
    mkdir(tabularium_base_url, name)
    base = tabularium_base_url.rstrip("/")
    requests.post(
        f"{base}/api/doc/{name}",
        json={
            "name": "index.md",
            "content": (
                "# Index\n\n"
                "[go to target](target.md)\n\n"
                "[external](http://example.com/page)\n\n"
                "[hash only](#section)\n\n"
                "[parent ref](../other.md)\n"
            ),
        },
        timeout=15,
    ).raise_for_status()
    requests.post(
        f"{base}/api/doc/{name}",
        json={"name": "target.md", "content": "# Target\n\nAquilas over the target."},
        timeout=15,
    ).raise_for_status()
    return name


def test_internal_relative_ref_href_is_resolved(
    selenium_driver, tabularium_base_url: str, ref_dir: str
):
    """Relative link `<a href>` must be the resolved ?open= URL, not raw `target.md`."""
    url = _open_doc_url(tabularium_base_url, ref_dir, "index.md")
    selenium_driver.get(url)
    _wait_app_ready(selenium_driver)

    # Wait for the markdown link to appear in the preview
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"),
        ),
    )

    links = selenium_driver.find_elements(
        By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"
    )
    # Find the "go to target" link
    target_link = next((a for a in links if a.text == "go to target"), None)
    assert target_link is not None, "Could not find 'go to target' link in preview"

    href = target_link.get_attribute("href")
    assert href is not None
    # Must be an absolute URL pointing to /entries/<dir> with ?open= param
    assert "/entries/" in href, f"Expected /entries/ in href, got: {href!r}"
    assert "open=" in href, f"Expected open= query param in href, got: {href!r}"
    assert "target.md" in href, f"Expected target.md in href, got: {href!r}"
    # Must NOT be just a relative path (raw href would resolve to /entries/target.md or similar)
    assert f"/{ref_dir}/" in href or f"%2F{ref_dir}%2F" in href or ref_dir in href, (
        f"Expected dir {ref_dir!r} in resolved href, got: {href!r}"
    )


def test_internal_relative_ref_click_navigates(
    selenium_driver, tabularium_base_url: str, ref_dir: str
):
    """Clicking a relative markdown link must open the target document in preview."""
    url = _open_doc_url(tabularium_base_url, ref_dir, "index.md")
    selenium_driver.get(url)
    _wait_app_ready(selenium_driver)

    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"),
        ),
    )

    links = selenium_driver.find_elements(
        By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"
    )
    target_link = next((a for a in links if a.text == "go to target"), None)
    assert target_link is not None, "Could not find 'go to target' link"

    target_link.click()

    # Preview pane should now show target.md content
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Aquilas over the target",
        ),
    )


def test_external_ref_href_passthrough(
    selenium_driver, tabularium_base_url: str, ref_dir: str
):
    """External http:// links must keep their original href unchanged."""
    url = _open_doc_url(tabularium_base_url, ref_dir, "index.md")
    selenium_driver.get(url)
    _wait_app_ready(selenium_driver)

    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"),
        ),
    )

    links = selenium_driver.find_elements(
        By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"
    )
    external_link = next((a for a in links if a.text == "external"), None)
    assert external_link is not None, "Could not find 'external' link"

    href = external_link.get_attribute("href")
    assert href == "http://example.com/page", f"External href mutated: {href!r}"


def test_hash_ref_href_passthrough(
    selenium_driver, tabularium_base_url: str, ref_dir: str
):
    """Hash-only links must keep their #anchor href unchanged."""
    url = _open_doc_url(tabularium_base_url, ref_dir, "index.md")
    selenium_driver.get(url)
    _wait_app_ready(selenium_driver)

    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"),
        ),
    )

    links = selenium_driver.find_elements(
        By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"
    )
    hash_link = next((a for a in links if a.text == "hash only"), None)
    assert hash_link is not None, "Could not find 'hash only' link"

    href = hash_link.get_attribute("href")
    assert href is not None
    assert href.endswith("#section"), f"Hash href mutated: {href!r}"


def test_resolved_href_is_directly_navigable(
    selenium_driver, tabularium_base_url: str, ref_dir: str
):
    """Copy-link scenario: navigate directly to the resolved href — target must open."""
    # First open index.md, grab the resolved href from the link
    url = _open_doc_url(tabularium_base_url, ref_dir, "index.md")
    selenium_driver.get(url)
    _wait_app_ready(selenium_driver)

    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"),
        ),
    )

    links = selenium_driver.find_elements(
        By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown a"
    )
    target_link = next((a for a in links if a.text == "go to target"), None)
    assert target_link is not None
    resolved_href = target_link.get_attribute("href")
    assert resolved_href

    # Now navigate directly to that URL (simulate copy-link + paste)
    selenium_driver.get(resolved_href)
    _wait_app_ready(selenium_driver)

    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane']"),
            "Aquilas over the target",
        ),
    )
