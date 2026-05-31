"""Installed agent implementation for benchmarking the Forge CLI agent."""

import json
import os
import shlex
import uuid
from pathlib import Path
from typing import Any

from harbor.agents.installed.base import BaseInstalledAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.agent.name import AgentName
from bench.forge_conversation import convert_events_to_trajectory


class ForgeAgent(BaseInstalledAgent):
    SUPPORTS_ATIF: bool = True

    @classmethod
    def get_forge_host_config_dir(cls) -> Path:
        """Get the Forge config directory from env, new default, or legacy path."""
        if "FORGE_CONFIG" in os.environ:
            return Path(os.environ["FORGE_CONFIG"]).expanduser()

        new_config_dir = Path.home() / ".forge"
        if new_config_dir.exists():
            return new_config_dir

        return Path.home() / "forge"

    FORGE_CONTAINER_CONFIG_DIR = "/root/.forge"

    @classmethod
    def get_forge_host_binary(cls) -> Path:
        """Get path to forge binary, from FORGE_BIN env var or discovered target output."""
        if "FORGE_BIN" in os.environ:
            configured_path = Path(os.environ["FORGE_BIN"]).expanduser()
            if configured_path.exists():
                return configured_path
            raise FileNotFoundError(
                f"FORGE_BIN is set but file does not exist: {configured_path}"
            )

        target_dir = Path.cwd() / "target"
        candidates = [target_dir / "release" / "forge"]
        candidates.extend(sorted(target_dir.glob("*/release/forge")))

        for candidate in candidates:
            if candidate.exists():
                return candidate

        raise FileNotFoundError(
            "Could not find forge binary. Expected one of: "
            + ", ".join(str(path) for path in candidates)
        )

    ALLOWED_TOOLS = [
        "read",
        "write",
        "fs_search",
        "sem_search",
        "remove",
        "patch",
        "undo",
        "shell",
        "fetch",
        "followup",
        "plan",
        "skill",
    ]

    def __init__(
        self,
        logs_dir: Path,
        max_thinking_tokens: int | None = None,
        *args,
        **kwargs,
    ):
        super().__init__(logs_dir, *args, **kwargs)
        self._max_thinking_tokens = max_thinking_tokens
        self._conversation_id: str | None = None
        self._task_timeout_secs: float | None = None

    @staticmethod
    def name() -> str:
        return AgentName.FORGE.value if hasattr(AgentName, "FORGE") else "forge"

    def _get_forge_env(self) -> dict[str, str]:
        """Get environment variables for forge execution."""
        env = {}


        # Tool timeout configuration (30 minutes for long-running benchmark tasks)
        env["FORGE_TOOL_TIMEOUT"] = "18000"  # 30 minutes in seconds

        # Run in background (non-interactive) mode for benchmarks
        env["FORGE_BACKGROUND"] = "true"
        env["FORGE_CONFIG"] = self.FORGE_CONTAINER_CONFIG_DIR

        # Pass task timeout so the agent can budget its time
        if self._task_timeout_secs is not None:
            env["FORGE_TASK_TIMEOUT_SECS"] = str(int(self._task_timeout_secs))

        # Pass through API keys
        api_key_vars = [
            "FORGE_API_KEY",
            "DEEPSEEK_API_KEY",
            "GITHUB_COPILOT_API_KEY",
            "OPENROUTER_API_KEY",
            "REQUESTY_API_KEY",
            "XAI_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "CLAUDE_API_KEY",
            "CEREBRAS_API_KEY",
            "ZAI_API_KEY",
            "ZAI_CODING_API_KEY",
            "BIG_MODEL_API_KEY",
            "VERTEX_AI_AUTH_TOKEN",
            "AZURE_API_KEY",
            "LLAMA_CPP_API_KEY",
            "VLLM_API_KEY",
            "JAN_AI_API_KEY",
            "OLLAMA_API_KEY",
            "LM_STUDIO_API_KEY",
            "IO_INTELLIGENCE_API_KEY",
            # URL Parameters
            "OPENAI_URL",
            "ANTHROPIC_URL",
            "PROJECT_ID",
            "LOCATION",
            "AZURE_RESOURCE_NAME",
            "AZURE_DEPLOYMENT_NAME",
            "AZURE_API_VERSION",
            "LLAMA_CPP_URL",
            "LLAMA_CPP_PORT",
            "VLLM_URL",
            "VLLM_PORT",
            "JAN_AI_URL",
            "JAN_AI_PORT",
            "OLLAMA_URL",
            "OLLAMA_PORT",
            "LM_STUDIO_URL",
            "LM_STUDIO_PORT",
            "FORGE_WORKSPACE_SERVER_URL",
            "AWS_REGION",
            # MCP server API keys
            "CONTEXT7_API_KEY",
        ]

        for key_var in api_key_vars:
            if key_var in os.environ:
                env[key_var] = os.environ[key_var]

        env["FORGE_LOG"] = "forge=info"

        return env

    async def install(self, environment: BaseEnvironment) -> None:
        """Install system dependencies and forge binary inside the container.

        Installs CA certificates, git, curl, coreutils via apt, then sets up
        a uv-managed Python environment with openai/pydantic, and finally
        uploads the forge binary.
        """
        # Install system packages (CA certs are critical for HTTPS API calls)
        await self.exec_as_root(
            environment,
            command=(
                "apt-get update > /dev/null 2>&1 && "
                "apt-get install -y curl git coreutils > /dev/null 2>&1 && "
                "command -v nohup >/dev/null 2>&1 || (echo 'nohup not found' && exit 1)"
            ),
        )

        # Install uv and create an isolated Python environment with deps.
        # We use uv instead of pip because some tasks intentionally break pip.
        # Some tasks also override /root so we install to /uv.
        await self.exec_as_root(
            environment,
            command=(
                "curl -LsSf https://astral.sh/uv/install.sh 2>/dev/null | env UV_INSTALL_DIR=/uv sh 2>/dev/null && "
                "export PATH=/uv:$PATH && "
                "export UV_PYTHON_INSTALL_DIR=/uv/python && "
                "export UV_CACHE_DIR=/uv/cache && "
                "uv venv /uv/forge --python 3.13 2>/dev/null && "
                ". /uv/forge/bin/activate && "
                "uv pip install openai pydantic 2>/dev/null"
            ),
        )

        # Ensure Python is discoverable in non-login/non-activated shells.
        # Only create symlinks if no python already exists — task containers
        # often ship their own Python with pre-installed dependencies
        # (pytest, numpy, etc.) and overwriting it would break those deps.
        await self.exec_as_root(
            environment,
            command=(
                "[ ! -e /usr/local/bin/python ] && "
                "ln -sf /uv/forge/bin/python /usr/local/bin/python; "
                "[ ! -e /usr/local/bin/python3 ] && "
                "ln -sf /uv/forge/bin/python /usr/local/bin/python3; "
                "true"
            ),
        )

        # Upload forge binary and make it executable
        await environment.upload_file(
            source_path=self.get_forge_host_binary(),
            target_path="/usr/local/bin/forge",
        )
        await self.exec_as_root(
            environment,
            command="chmod +x /usr/local/bin/forge",
        )

        # Verify forge is installed (suppress output)
        await self.exec_as_agent(
            environment,
            command="/usr/local/bin/forge --version > /dev/null 2>&1 || true",
        )

    async def setup(self, environment: BaseEnvironment) -> None:
        """Set up forge agent: install binary, upload config files."""
        # Parent setup creates /installed-agent dir, calls self.install(),
        # and auto-detects version
        await super().setup(environment)

        # Create forge config directory in container
        await self.exec_as_agent(
            environment,
            command=f"mkdir -p {self.FORGE_CONTAINER_CONFIG_DIR}",
        )

        # Upload forge config files (credentials and config)
        config_dir = self.get_forge_host_config_dir()
        config_file = config_dir / ".forge.toml"
        credentials_file = config_dir / ".credentials.json"

        if config_file.exists():
            await environment.upload_file(
                source_path=config_file,
                target_path=f"{self.FORGE_CONTAINER_CONFIG_DIR}/.forge.toml",
            )
        if credentials_file.exists():
            await environment.upload_file(
                source_path=credentials_file,
                target_path=f"{self.FORGE_CONTAINER_CONFIG_DIR}/.credentials.json",
            )

        # Upload MCP configuration so MCP servers (context7, deepwiki, etc.) are available
        mcp_config_file = config_dir / ".mcp.json"
        if mcp_config_file.exists():
            await environment.upload_file(
                source_path=mcp_config_file,
                target_path=f"{self.FORGE_CONTAINER_CONFIG_DIR}/.mcp.json",
            )

        config_path = self.logs_dir / "config.json"
        if config_path.exists():
            try:
                with open(config_path, "r") as f:
                    config_data = json.load(f)
                # Resolve and store the effective agent timeout so Forge
                # can budget its time.  The timeout is defined per-task in
                # task.toml ([agent] timeout_sec) and may be overridden or
                # scaled by the trial config.
                self._resolve_task_timeout(config_data)
            except Exception as e:
                self.logger.warning(f"Failed to load specific agent md from payload: {e}")

    def _resolve_task_timeout(self, config_data: dict) -> None:
        """Compute the effective agent timeout from trial config + task.toml.

        Harbor calculates the timeout as:
            min(override_timeout or task_timeout, max_timeout) * multiplier

        The task.toml file (which contains [agent] timeout_sec) lives in
        Harbor's task cache.  We locate it via the task path recorded in
        config.json and search well-known cache directories.
        """
        try:
            import tomllib
        except ModuleNotFoundError:
            # Python < 3.11 fallback
            try:
                import tomli as tomllib  # type: ignore[no-redef]
            except ModuleNotFoundError:
                return

        # 1. Check for an explicit override in the trial config
        agent_cfg = config_data.get("agent", {})
        override = agent_cfg.get("override_timeout_sec")
        if override is not None:
            max_timeout = agent_cfg.get("max_timeout_sec") or float("inf")
            multiplier = (
                config_data.get("agent_timeout_multiplier")
                or config_data.get("timeout_multiplier")
                or 1.0
            )
            self._task_timeout_secs = min(override, max_timeout) * multiplier
            return

        # 2. Find task.toml in the harbor cache
        task_path = config_data.get("task", {}).get("path")
        if not task_path:
            return

        cache_bases = [
            Path.home() / ".cache" / "harbor" / "tasks",
            Path("/tmp") / "harbor" / "tasks",
        ]

        task_toml_path = None
        for cache_base in cache_bases:
            if not cache_base.is_dir():
                continue
            # Harbor stores tasks under <cache>/tasks/<hash>/<task_path>/task.toml
            for candidate in cache_base.iterdir():
                toml_candidate = candidate / task_path / "task.toml"
                if toml_candidate.is_file():
                    task_toml_path = toml_candidate
                    break
            if task_toml_path:
                break

        if task_toml_path is None:
            return

        # 3. Parse the timeout from task.toml
        with open(task_toml_path, "rb") as f:
            task_config = tomllib.load(f)

        base_timeout = task_config.get("agent", {}).get("timeout_sec", 600.0)
        max_timeout = agent_cfg.get("max_timeout_sec") or float("inf")
        multiplier = (
            config_data.get("agent_timeout_multiplier")
            or config_data.get("timeout_multiplier")
            or 1.0
        )
        self._task_timeout_secs = min(base_timeout, max_timeout) * multiplier

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        """Execute forge with the given instruction inside the container."""
        escaped_instruction = shlex.quote(instruction)
        env = self._get_forge_env()

        # Generate a unique conversation ID for tracking
        self._conversation_id = str(uuid.uuid4())
        env["_FORGE_CONVERSATION_ID"] = self._conversation_id

        # Run forge with the instruction
        await self.exec_as_agent(
            environment,
            command=(
                f"/usr/local/bin/forge "
                f"--conversation-id {self._conversation_id} "
                f"-p {escaped_instruction} > /logs/agent/forge-output.txt 2>&1"
            ),
            env=env,
        )

        # Dump conversation as JSON in the agent logs directory
        await self.exec_as_agent(
            environment,
            command=(
                f"/usr/local/bin/forge "
                f"conversation "
                f"dump "
                f"{self._conversation_id}"
            ),
            env=env,
            cwd="/logs/agent",
        )

        # Dump conversation as HTML in the agent logs directory
        await self.exec_as_agent(
            environment,
            command=(
                f"/usr/local/bin/forge "
                f"conversation "
                f"dump "
                f"{self._conversation_id} "
                f"--html"
            ),
            env=env,
            cwd="/logs/agent",
        )

        # Files are now available in the logs directory, no need to cat them

    def populate_context_post_run(self, context: AgentContext) -> None:
        """Parse forge trajectory and populate context with token metrics."""
        # Look for session logs in the logs directory
        session_dir = self.logs_dir
        trajectory = convert_events_to_trajectory(session_dir, self.model_name)

        if trajectory is not None:
            # Write trajectory to the standard location
            trajectory_path = self.logs_dir / "trajectory.json"
            try:
                with open(trajectory_path, "w", encoding="utf-8") as handle:
                    json.dump(
                        trajectory.to_json_dict(), handle, indent=2, ensure_ascii=False
                    )
            except Exception as e:
                print(f"Failed to write trajectory to {trajectory_path}: {e}")

            # Populate context with token metrics from the trajectory
            metrics = trajectory.final_metrics
            if metrics:
                if metrics.total_cost_usd is not None:
                    context.cost_usd = metrics.total_cost_usd
                context.n_input_tokens = metrics.total_prompt_tokens or 0
                context.n_cache_tokens = metrics.total_cached_tokens or 0
                context.n_output_tokens = metrics.total_completion_tokens or 0
