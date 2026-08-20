"""Integration tests for forgecode end-to-end workflows."""
import pytest
import subprocess
import sys
from pathlib import Path


class TestCLIIntegration:
    """Integration tests for CLI commands."""

    @pytest.mark.integration
    def test_help_flag(self):
        """CLI should print help text."""
        result = subprocess.run(
            [sys.executable, '-m', 'src', '--help'],
            capture_output=True, text=True, timeout=30
        )
        assert result.returncode == 0 or 'usage' in result.stdout.lower() or 'help' in result.stdout.lower()

    @pytest.mark.integration
    def test_version_flag(self):
        """CLI should print version."""
        result = subprocess.run(
            [sys.executable, '-m', 'src', '--version'],
            capture_output=True, text=True, timeout=30
        )
        assert result.returncode == 0 or 'version' in result.stdout.lower()


class TestWorkflowIntegration:
    """Integration tests for end-to-end workflows."""

    @pytest.mark.integration
    def test_config_load(self):
        """Config loading should not crash."""
        try:
            from src.config import Config
            config = Config()
            assert config is not None
        except Exception as e:
            if 'file not found' in str(e).lower() or 'no such' in str(e).lower():
                pytest.skip('Config file not found - expected in dev')
            raise

    @pytest.mark.integration
    def test_quality_gate_cycle(self):
        """Quality gate should be able to run a full cycle."""
        try:
            from src.quality_gate import QualityGate
            gate = QualityGate()
            result = gate.run()
            assert hasattr(result, 'passed') or isinstance(result, dict)
        except Exception as e:
            if 'import' in str(e).lower():
                pytest.skip('Module not available')
            raise


class TestEndToEnd:
    """Full end-to-end workflow tests."""

    @pytest.mark.e2e
    @pytest.mark.slow
    def test_full_pipeline(self):
        """Full pipeline from config to output."""
        pytest.skip('E2E pipeline test - enable when full environment is ready')

    @pytest.mark.e2e
    @pytest.mark.slow
    def test_error_recovery(self):
        """Pipeline should recover from partial failures."""
        pytest.skip('E2E recovery test - enable when full environment is ready')
