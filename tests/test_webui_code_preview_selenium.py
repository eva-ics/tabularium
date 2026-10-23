"""Code previews: extension fallback, MIME precedence, exact source, inert HTML."""
import uuid
from urllib.parse import quote
import pytest
import requests
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait

pytestmark = pytest.mark.webui


def open_preview(driver, base, name, content, mime=None):
    root = "code_preview_" + uuid.uuid4().hex[:10]
    requests.post(base + "/api/doc", json={"path": "/" + root}, timeout=10).raise_for_status()
    path = "/" + root + "/" + name
    requests.put(base + "/api/doc" + path, json={"content": content}, timeout=10).raise_for_status()
    driver.get(base + "/entries/" + root)
    WebDriverWait(driver, 20).until(lambda d: d.find_element(By.TAG_NAME, "body").get_attribute("data-tabularium-ready") == "true")
    if mime is not None:
        # Exercise explicit API metadata, independently of filename inference.
        driver.execute_script("""
            const original = window.fetch.bind(window);
            const path = arguments[0], mime = arguments[1];
            window.fetch = async (...args) => {
              const response = await original(...args);
              const url = new URL(typeof args[0] === 'string' ? args[0] : args[0].url, location.href);
              if (url.pathname === '/api/doc' + path && response.ok) {
                const body = await response.clone().json();
                return new Response(JSON.stringify({...body, mime_type: mime}), {
                  status: response.status, headers: response.headers
                });
              }
              return response;
            };
        """, path, mime)
    row = WebDriverWait(driver, 20).until(lambda d: d.find_element(By.CSS_SELECTOR, f"li[data-entry-name='{name}']"))
    row.click()
    return WebDriverWait(driver, 20)


@pytest.mark.parametrize("name,language,content", [
    ("config.json", "json", '{"large": 9007199254740993, "html": "<img src=x onerror=alert(1)>"}\n'),
    ("run.sh", "bash", '#!/bin/sh\n# not a heading\necho "<img src=x onerror=alert(1)>"\n'),
])
def test_code_preview_preserves_source_and_raw_toggle(selenium_driver, tabularium_base_url, name, language, content):
    d = selenium_driver
    wait = open_preview(d, tabularium_base_url, name, content)
    code = wait.until(lambda d: d.find_element(By.CSS_SELECTOR, "[data-testid='preview-code']"))
    assert code.get_attribute("data-language") == language
    assert code.get_attribute("textContent") == content
    assert code.find_elements(By.CSS_SELECTOR, "span[class*='hljs-']")
    assert not code.find_elements(By.CSS_SELECTOR, "img, script, iframe")
    d.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    wait.until(lambda d: not d.find_elements(By.CSS_SELECTOR, "[data-testid='preview-code']"))
    raw = wait.until(lambda d: d.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane'] pre"))
    assert raw.get_attribute("textContent") == content
    d.find_element(By.CSS_SELECTOR, "[data-testid='preview-raw-toggle']").click()
    wait.until(lambda d: d.find_element(By.CSS_SELECTOR, "[data-testid='preview-code']"))


@pytest.mark.parametrize("name,mime,expected", [
    ("payload.md", "Application/JSON; charset=utf-8", "json"),
    ("payload.json", "text/x-shellscript", "bash"),
    ("payload.sh", "text/markdown", "markdown"),
    ("payload.unknown", None, "markdown"),
    ("payload.txt", None, "text"),
    ("payload.list", None, "text"),
    ("payload.md", "text/plain", "text"),
])
def test_preview_mime_precedence_and_markdown_default(selenium_driver, tabularium_base_url, name, mime, expected):
    d = selenium_driver
    content = "# Preserved heading\n\n**bold**\n" if expected == "markdown" else '{"enabled": true}\n'
    wait = open_preview(d, tabularium_base_url, name, content, mime)
    if expected == "markdown":
        heading = wait.until(lambda d: d.find_element(By.CSS_SELECTOR, "[data-testid='preview-pane'] h1"))
        assert heading.text == "Preserved heading"
        assert not d.find_elements(By.CSS_SELECTOR, "[data-testid='preview-code']")
    else:
        code = wait.until(lambda d: d.find_element(By.CSS_SELECTOR, "[data-testid='preview-code']"))
        assert code.get_attribute("data-language") == expected
