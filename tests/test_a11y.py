"""
Tests for the accessibility (a11y) module.
"""

import sys
import os

# Add src to path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from a11y import (
    ScreenReaderDescription,
    ContrastChecker,
    SemanticStructure,
    A11yAudit,
    a11y_description,
)


class TestScreenReaderDescription:
    def test_describe_status(self):
        sr = ScreenReaderDescription()
        result = sr.describe_status("success", "Build passed")
        assert "Status:" in result
        assert "success" in result
        assert "Build passed" in result

    def test_describe_table(self):
        sr = ScreenReaderDescription()
        result = sr.describe_table(["Name", "Status"], 5)
        assert "Table:" in result
        assert "2 columns" in result
        assert "5 rows" in result

    def test_describe_progress(self):
        sr = ScreenReaderDescription()
        result = sr.describe_progress(3, 10, "Tests")
        assert "Tests:" in result
        assert "30%" in result
        assert "3/10" in result

    def test_describe_error(self):
        sr = ScreenReaderDescription()
        result = sr.describe_error("Type", "Something went wrong")
        assert "Error:" in result
        assert "Type" in result

    def test_caching(self):
        sr = ScreenReaderDescription()
        r1 = sr.describe("test", "hello")
        r2 = sr.describe("test", "hello")
        assert r1 == r2


class TestContrastChecker:
    def test_relative_luminance_black(self):
        l = ContrastChecker.relative_luminance(0, 0, 0)
        assert l == 0.0

    def test_relative_luminance_white(self):
        l = ContrastChecker.relative_luminance(255, 255, 255)
        assert l > 0.9

    def test_contrast_ratio_black_white(self):
        ratio = ContrastChecker.contrast_ratio((0, 0, 0), (255, 255, 255))
        assert abs(ratio - 21.0) < 0.1

    def test_meets_aa_normal_text(self):
        assert ContrastChecker.meets_aa(4.5)
        assert ContrastChecker.meets_aa(7.0)
        assert not ContrastChecker.meets_aa(3.0)

    def test_meets_aa_large_text(self):
        assert ContrastChecker.meets_aa(4.5, large_text=True)
        assert not ContrastChecker.meets_aa(3.0, large_text=True)

    def test_meets_aaa(self):
        assert ContrastChecker.meets_aaa(7.0)
        assert not ContrastChecker.meets_aaa(5.0)


class TestSemanticStructure:
    def test_valid_structure(self):
        elements = [
            {"tag": "main"},
            {"tag": "h1", "text": "Title"},
            {"tag": "p", "text": "Content"},
        ]
        issues = SemanticStructure.validate_structure(elements)
        assert len(issues) == 0

    def test_missing_landmark(self):
        elements = [{"tag": "p", "text": "No landmark"}]
        issues = SemanticStructure.validate_structure(elements)
        assert any("Missing document/main landmark" in i for i in issues)

    def test_heading_hierarchy(self):
        elements = [
            {"tag": "main"},
            {"tag": "h3", "text": "Skipped h1"},
        ]
        issues = SemanticStructure.validate_structure(elements)
        assert any("h3" in i and "expected h1" in i for i in issues)

    def test_image_missing_alt(self):
        elements = [
            {"tag": "main"},
            {"tag": "h1", "text": "Title"},
            {"tag": "img"},
        ]
        issues = SemanticStructure.validate_structure(elements)
        assert any("Image missing alt" in i for i in issues)

    def test_interactive_no_label(self):
        elements = [
            {"tag": "main"},
            {"tag": "h1", "text": "Title"},
            {"tag": "button"},
        ]
        issues = SemanticStructure.validate_structure(elements)
        assert any("button" in i and "missing accessible name" in i for i in issues)


class TestA11yAudit:
    def test_passing_audit(self):
        elements = [
            {"tag": "main"},
            {"tag": "h1", "text": "Title"},
            {"tag": "button", "text": "Click me"},
        ]
        audit = A11yAudit()
        result = audit.audit(elements)
        assert result["pass"]

    def test_failing_audit(self):
        elements = [{"tag": "p", "text": "No landmark"}]
        audit = A11yAudit()
        result = audit.audit(elements)
        assert not result["pass"]
        assert len(result["structural_issues"]) > 0

    def test_contrast_audit(self):
        elements = [{"tag": "main"}, {"tag": "h1", "text": "OK"}]
        colors = [(0, 0, 0), (10, 10, 10)]  # Very low contrast
        audit = A11yAudit()
        result = audit.audit(elements, colors=colors)
        assert not result["pass"]
        assert len(result["contrast_issues"]) > 0


class TestConvenienceFunctions:
    def test_a11y_description(self):
        result = a11y_description("status", "running")
        assert "Status:" in result
        assert "running" in result
