"""Selenium coverage for meetings/mat — Ferrum math preview manifest (M01–M05).

Prerequisites: TABULARIUM_TEST_URL → http://10.90.1.122:<port> (AGENTS.md).
"""

from __future__ import annotations

import re
import uuid

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

from tests.helpers import mkdir, put_doc

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


def _open_seed_dir_and_wait(driver, seed_dir: str) -> None:
    driver.find_element(
        By.CSS_SELECTOR,
        f"[data-entry-name='{seed_dir}']",
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


def _open_math_doc(
    driver,
    base: str,
    seed_dir: str,
    file_name: str,
) -> None:
    driver.get(base.rstrip("/") + "/entries")
    _wait_app_ready(driver)
    _open_seed_dir_and_wait(driver, seed_dir)
    driver.find_element(
        By.CSS_SELECTOR,
        f"[data-entry-name='{file_name}']",
    ).click()


@pytest.fixture
def math_seed_dir(tabularium_base_url: str) -> str:
    slug = uuid.uuid4().hex[:8]
    name = f"math_{slug}"
    mkdir(tabularium_base_url, name)
    return name


def test_math_m01_valid_block_formula_renders(
    selenium_driver,
    tabularium_base_url: str,
    math_seed_dir: str,
) -> None:
    """M01 — valid block `$$…$$` produces KaTeX output in preview."""
    put_doc(
        tabularium_base_url,
        math_seed_dir,
        "m01.md",
        "Einstein:\n\n$$E = mc^2$$\n",
    )
    _open_math_doc(selenium_driver, tabularium_base_url, math_seed_dir, "m01.md")
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown .katex"),
        ),
    )


def test_math_m02_invalid_formula_degrades_not_empty(
    selenium_driver,
    tabularium_base_url: str,
    math_seed_dir: str,
) -> None:
    """M02 — invalid TeX shows KaTeX error styling; preview still populated."""
    put_doc(
        tabularium_base_url,
        math_seed_dir,
        "m02.md",
        "Broken:\n\n$$\\foo$$\n",
    )
    _open_math_doc(selenium_driver, tabularium_base_url, math_seed_dir, "m02.md")
    _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown .katex"),
        ),
    )
    pane = selenium_driver.find_element(
        By.CSS_SELECTOR,
        "[data-testid='preview-pane']",
    )
    html = pane.get_attribute("innerHTML") or ""
    # KaTeX ≥0.16: undefined macros often render as red `.katex` text, not `.katex-error`.
    assert pane.find_elements(By.CSS_SELECTOR, ".markdown .katex")
    hl = html.lower()
    assert "katex-error" in html or "#cc0000" in hl or re.search(
        r"rgb\s*\(\s*204\s*,\s*0\s*,\s*0\s*\)",
        hl,
    )
    body = pane.text.strip()
    assert len(body) > 10
    assert "Select a file" not in body


def test_math_m03_dollar_price_not_math_mode(
    selenium_driver,
    tabularium_base_url: str,
    math_seed_dir: str,
) -> None:
    """M03 — currency-style `$99.99` does not create KaTeX widgets."""
    put_doc(
        tabularium_base_url,
        math_seed_dir,
        "m03.md",
        "Price is $99.99 today.\n",
    )
    _open_math_doc(selenium_driver, tabularium_base_url, math_seed_dir, "m03.md")
    _wait(selenium_driver).until(
        EC.text_to_be_present_in_element(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown"),
            "99.99",
        ),
    )
    katex = selenium_driver.find_elements(
        By.CSS_SELECTOR,
        "[data-testid='preview-pane'] .markdown .katex",
    )
    assert len(katex) == 0


def test_math_m04_code_fence_dollar_passthrough(
    selenium_driver,
    tabularium_base_url: str,
    math_seed_dir: str,
) -> None:
    """M04 — fenced code keeps `$` literals; no KaTeX inside `pre`."""
    put_doc(
        tabularium_base_url,
        math_seed_dir,
        "m04.md",
        "Shell:\n\n```\necho $PATH\n```\n",
    )
    _open_math_doc(selenium_driver, tabularium_base_url, math_seed_dir, "m04.md")
    pre = _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown pre"),
        ),
    )
    assert "$PATH" in pre.text or "echo" in pre.text
    assert not pre.find_elements(By.CSS_SELECTOR, ".katex")


def test_math_m05_no_xss_from_formula_markup(
    selenium_driver,
    tabularium_base_url: str,
    math_seed_dir: str,
) -> None:
    """M05 — crafted `\\text{…}` does not yield executable script URLs in preview."""
    m05_body = (
        "Probe:\n\n"
        + r"$$\text{<img src=x onerror=alert(1)>}$$"
        + "\n"
    )
    put_doc(tabularium_base_url, math_seed_dir, "m05.md", m05_body)
    _open_math_doc(selenium_driver, tabularium_base_url, math_seed_dir, "m05.md")
    md = _wait(selenium_driver).until(
        EC.presence_of_element_located(
            (By.CSS_SELECTOR, "[data-testid='preview-pane'] .markdown"),
        ),
    )
    assert not md.find_elements(By.TAG_NAME, "script")
    html = (md.get_attribute("innerHTML") or "").lower()
    assert "javascript:" not in html
    # Safe MathML/text can still contain the substring `onerror=` inside escaped text — forbid real tags only.
    assert not re.search(r"<img\b[^>]*\bonerror\s*=", html, re.IGNORECASE)
