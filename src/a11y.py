"""
Accessibility (a11y) module for forgecode.

Implements WCAG 2.1 AA compliance helpers, screen-reader descriptions,
contrast checking, and semantic structure validation.

Pillar 16: Accessibility
"""

import json
import os
from typing import Any, Dict, List, Optional


class ScreenReaderDescription:
    """Generates screen-reader-compatible descriptions for CLI output."""

    def __init__(self, locale: str = "en"):
        self.locale = locale
        self._descriptions: Dict[str, str] = {}

    def describe(self, element: str, context: Optional[str] = None) -> str:
        """Generate a screen-reader description for an element."""
        key = f"{element}:{context}" if context else element
        if key in self._descriptions:
            return self._descriptions[key]

        patterns = {
            "status": "Status: {context}",
            "progress": "Progress: {context}",
            "error": "Error: {context}",
            "warning": "Warning: {context}",
            "success": "Success: {context}",
            "file": "File: {context}",
            "directory": "Directory: {context}",
            "command": "Command: {context}",
            "table": "Table: {context}",
            "code": "Code block: {context}",
            "heading": "Section: {context}",
            "list": "List: {context}",
            "link": "Link: {context}",
            "button": "Button: {context}",
        }

        pattern_key = element.split(".")[0] if "." in element else element
        pattern = patterns.get(pattern_key, "{context}")
        description = pattern.format(context=context or element)
        self._descriptions[key] = description
        return description

    def describe_status(self, status: str, details: str) -> str:
        """Describe a status indicator."""
        return self.describe("status", f"{status}: {details}")

    def describe_table(self, headers: List[str], row_count: int) -> str:
        """Describe a data table."""
        cols = ", ".join(headers)
        return self.describe("table", f"{len(headers)} columns ({cols}), {row_count} rows")

    def describe_progress(self, current: int, total: int, label: str = "") -> str:
        """Describe a progress indicator."""
        pct = (current / total * 100) if total > 0 else 0
        desc = f"{pct:.0f}% complete ({current}/{total})"
        if label:
            desc = f"{label}: {desc}"
        return self.describe("progress", desc)

    def describe_error(self, error_type: str, message: str) -> str:
        """Describe an error condition."""
        return self.describe("error", f"{error_type}: {message}")


class ContrastChecker:
    """WCAG 2.1 contrast ratio checker."""

    @staticmethod
    def relative_luminance(r: float, g: float, b: float) -> float:
        """Calculate relative luminance per WCAG 2.1."""
        def linearize(c: float) -> float:
            c /= 255.0
            return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
        return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)

    @staticmethod
    def contrast_ratio(color1: tuple, color2: tuple) -> float:
        """Calculate contrast ratio between two RGB colors."""
        l1 = ContrastChecker.relative_luminance(*color1)
        l2 = ContrastChecker.relative_luminance(*color2)
        lighter = max(l1, l2)
        darker = min(l1, l2)
        return (lighter + 0.05) / (darker + 0.05)

    @staticmethod
    def meets_aa(ratio: float, large_text: bool = False) -> bool:
        """Check if ratio meets WCAG AA requirements."""
        return ratio >= 7.0 if large_text else ratio >= 4.5

    @staticmethod
    def meets_aaa(ratio: float, large_text: bool = False) -> bool:
        """Check if ratio meets WCAG AAA requirements."""
        return ratio >= 4.5 if large_text else ratio >= 7.0


class SemanticStructure:
    """Validates semantic HTML/CLI structure for accessibility."""

    REQUIRED_SEMANTICS = ["document", "heading", "navigation", "main", "region"]

    @classmethod
    def validate_structure(cls, elements: List[Dict[str, Any]]) -> List[str]:
        """Validate semantic structure and return issues found."""
        issues = []
        tag_names = [e.get("tag", "") for e in elements]

        if "document" not in tag_names and "main" not in tag_names:
            issues.append("Missing document/main landmark")

        headings = [e for e in elements if e.get("tag", "").startswith("h")]
        if not headings:
            issues.append("No headings found - document lacks structure")
        else:
            levels = [int(h["tag"][1]) for h in headings if len(h["tag"]) > 1]
            if levels and levels[0] != 1:
                issues.append(f"First heading is h{levels[0]}, expected h1")

        images = [e for e in elements if e.get("tag") == "img"]
        for img in images:
            if not img.get("alt"):
                issues.append("Image missing alt text")

        interactive = {"button", "a", "input", "select", "textarea"}
        for elem in elements:
            if elem.get("tag") in interactive and not elem.get("aria-label") and not elem.get("text"):
                issues.append(f"Interactive <{elem['tag']}> missing accessible name")

        return issues


class A11yAudit:
    """Run a full a11y audit on a set of elements."""

    def __init__(self):
        self.screen_reader = ScreenReaderDescription()
        self.contrast = ContrastChecker()
        self.structure = SemanticStructure()
        self.results: Dict[str, Any] = {
            "contrast_issues": [],
            "structural_issues": [],
            "screen_reader_issues": [],
            "pass": True,
        }

    def audit(self, elements: List[Dict[str, Any]], colors: Optional[List[tuple]] = None) -> Dict[str, Any]:
        """Run full a11y audit and return results."""
        self.results["structural_issues"] = self.structure.validate_structure(elements)

        if colors:
            for i in range(0, len(colors) - 1, 2):
                ratio = self.contrast.contrast_ratio(colors[i], colors[i + 1])
                if not self.contrast.meets_aa(ratio):
                    self.results["contrast_issues"].append({
                        "colors": [colors[i], colors[i + 1]],
                        "ratio": round(ratio, 2),
                        "level": "fail",
                    })

        self.results["pass"] = (
            len(self.results["structural_issues"]) == 0
            and len(self.results["contrast_issues"]) == 0
            and len(self.results["screen_reader_issues"]) == 0
        )
        return self.results


def load_locale(locale_dir: str, locale: str = "en") -> Dict[str, str]:
    """Load an a11n locale file."""
    path = os.path.join(locale_dir, f"{locale}.json")
    if not os.path.exists(path):
        return {}
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def a11y_description(element: str, context: Optional[str] = None, locale: str = "en") -> str:
    """Convenience function to get a screen-reader description."""
    sr = ScreenReaderDescription(locale=locale)
    return sr.describe(element, context)
