"""Pytest hooks — optional REST suite vs Web UI Selenium (base URL env)."""

from __future__ import annotations

import os
from urllib.parse import urlparse

import pytest

# Selenium Web UI tests MUST target this host only (Enginseer doctrine — no loopback).
SELENIUM_TEST_HOST = "10.90.1.122"


def pytest_configure(config) -> None:
    config.addinivalue_line(
        "markers",
        "webui: Selenium tests for embedded React UI (needs Chrome + base URL)",
    )


def _webui_base_url_raw() -> str:
    return (
        os.environ.get("TABULARIUM_TEST_URL") or os.environ.get("TABULARIUM_URL") or ""
    ).rstrip("/")


def _url_hostname(url: str) -> str | None:
    try:
        return urlparse(url).hostname
    except ValueError:
        return None


def pytest_collection_modifyitems(config, items) -> None:
    for item in items:
        if item.get_closest_marker("webui"):
            base = _webui_base_url_raw()
            if not base:
                item.add_marker(
                    pytest.mark.skip(
                        reason="set TABULARIUM_TEST_URL or TABULARIUM_URL for Web UI tests",
                    ),
                )
                continue
            host = _url_hostname(base)
            if host != SELENIUM_TEST_HOST:
                item.add_marker(
                    pytest.mark.skip(
                        reason=(
                            f"Web UI Selenium must use http://{SELENIUM_TEST_HOST}:<port> "
                            f"(got host {host!r}); see AGENTS.md"
                        ),
                    ),
                )
            continue
        if not os.getenv("TABULARIUM_TEST_URL"):
            item.add_marker(
                pytest.mark.skip(
                    reason="set TABULARIUM_TEST_URL to enable (e.g. http://127.0.0.1:3050)",
                ),
            )


def _tabularium_base_url() -> str:
    return _webui_base_url_raw()


@pytest.fixture
def tabularium_base_url() -> str:
    base = _tabularium_base_url()
    if not base:
        pytest.skip("TABULARIUM_TEST_URL or TABULARIUM_URL required")
    if _url_hostname(base) != SELENIUM_TEST_HOST:
        pytest.skip(
            f"Web UI Selenium requires host {SELENIUM_TEST_HOST} (see AGENTS.md)",
        )
    return base


def _create_selenium_driver():
    try:
        from selenium import webdriver
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service
        from webdriver_manager.chrome import ChromeDriverManager
        from webdriver_manager.core.os_manager import ChromeType
    except ImportError:
        return None
    options = Options()
    options.add_argument("--headless=new")
    options.add_argument("--no-sandbox")
    options.add_argument("--disable-dev-shm-usage")
    options.add_argument("--disable-gpu")
    if os.environ.get("CI") or os.environ.get("TABULARIUM_CI"):
        for chrome_bin in ("/usr/bin/chromium-browser", "/usr/bin/chromium"):
            if os.path.isfile(chrome_bin):
                options.binary_location = chrome_bin
                break
        chromedriver = os.environ.get("CHROMEDRIVER_PATH")
        if not chromedriver or not os.path.isfile(chromedriver):
            chromedriver = next(
                (
                    p
                    for p in (
                        "/usr/bin/chromedriver",
                        "/usr/lib/chromium/chromedriver",
                    )
                    if os.path.isfile(p)
                ),
                None,
            )
        if chromedriver:
            service = Service(chromedriver)
        else:
            try:
                service = Service(ChromeDriverManager(chrome_type=ChromeType.CHROMIUM).install())
            except OSError:
                return None
    else:
        try:
            service = Service(ChromeDriverManager().install())
        except OSError:
            return None
    try:
        return webdriver.Chrome(service=service, options=options)
    except OSError:
        return None


@pytest.fixture
def selenium_driver():
    """Headless Chrome/Chromium for Web UI tests."""
    driver = _create_selenium_driver()
    if driver is None:
        pytest.skip("Selenium Chrome driver not available")
    yield driver
    driver.quit()
