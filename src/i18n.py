"""Internationalization scaffolding for forgecode.

This module provides the foundation for multi-language support.
Currently English-only; other locales can be added as JSON files in locales/.
"""

import json
from pathlib import Path
from typing import Optional

# Default locale
DEFAULT_LOCALE = "en"

# Supported locales
SUPPORTED_LOCALES = ["en"]

# Locale directory
LOCALE_DIR = Path(__file__).parent / "locales"


class I18n:
    """Simple i18n manager for forgecode.

    Usage:
        i18n = I18n()
        print(i18n.t("welcome"))
        print(i18n.t("errors.file_not_found", path="/foo/bar"))
    """

    def __init__(self, locale: Optional[str] = None):
        self.locale = locale or DEFAULT_LOCALE
        self._translations: dict = {}
        self._load_translations()

    def _load_translations(self) -> None:
        """Load translations for the current locale."""
        locale_file = LOCALE_DIR / f"{self.locale}.json"
        if locale_file.exists():
            with open(locale_file, encoding="utf-8") as f:
                self._translations = json.load(f)

        # Fallback: load default locale
        if self.locale != DEFAULT_LOCALE:
            default_file = LOCALE_DIR / f"{DEFAULT_LOCALE}.json"
            if default_file.exists():
                with open(default_file, encoding="utf-8") as f:
                    default_translations = json.load(f)
                # Merge: current locale takes precedence
                for key, value in default_translations.items():
                    if key not in self._translations:
                        self._translations[key] = value

    def t(self, key: str, **kwargs) -> str:
        """Translate a key with optional interpolation.

        Args:
            key: Dot-separated translation key (e.g. "errors.file_not_found")
            **kwargs: Template variables (e.g. path="/foo")

        Returns:
            Translated string, or the key itself if not found.
        """
        parts = key.split(".")
        value = self._translations
        for part in parts:
            if isinstance(value, dict):
                value = value.get(part)
            else:
                return key

        if value is None:
            return key

        if isinstance(value, str) and kwargs:
            try:
                return value.format(**kwargs)
            except (KeyError, IndexError):
                return value
        return value

    def set_locale(self, locale: str) -> None:
        """Change the active locale."""
        if locale in SUPPORTED_LOCALES:
            self.locale = locale
            self._translations = {}
            self._load_translations()


# Global instance
_i18n: Optional[I18n] = None


def get_i18n(locale: Optional[str] = None) -> I18n:
    """Get or create the global i18n instance."""
    global _i18n
    if _i18n is None:
        _i18n = I18n(locale)
    elif locale is not None and locale != _i18n.locale:
        _i18n.set_locale(locale)
    return _i18n


def t(key: str, **kwargs) -> str:
    """Shorthand translation function."""
    return get_i18n().t(key, **kwargs)
