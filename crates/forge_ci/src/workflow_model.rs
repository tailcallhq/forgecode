//! Minimal ordered GitHub Actions model used by the Forge CI workflow
//! generator.

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Default, Serialize)]
pub(crate) struct Workflow {
    name: String,
    #[serde(rename = "on")]
    event: Event,
    #[serde(skip_serializing_if = "Permissions::is_empty")]
    permissions: Permissions,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    env: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    concurrency: Option<Concurrency>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    jobs: IndexMap<String, Job>,
}

impl Workflow {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Self::default() }
    }

    pub(crate) fn on(mut self, event: Event) -> Self {
        self.event = event;
        self
    }

    pub(crate) fn permissions(mut self, permissions: Permissions) -> Self {
        self.permissions = permissions;
        self
    }

    pub(crate) fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub(crate) fn concurrency(
        mut self,
        group: impl Into<String>,
        cancel_in_progress: bool,
    ) -> Self {
        self.concurrency = Some(Concurrency { group: group.into(), cancel_in_progress });
        self
    }

    pub(crate) fn add_job(mut self, id: impl Into<String>, job: Job) -> Self {
        self.jobs.insert(id.into(), job);
        self
    }

    pub(crate) fn to_yaml(&self) -> Result<String, serde_yaml_ng::Error> {
        serde_yaml_ng::to_string(self)
    }
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Event {
    #[serde(skip_serializing_if = "Option::is_none")]
    push: Option<Push>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    schedule: Vec<Schedule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pull_request: Option<PullRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pull_request_target: Option<PullRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issues: Option<Issues>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<Release>,
}

impl Event {
    pub(crate) fn push(mut self, push: Push) -> Self {
        self.push = Some(push);
        self
    }

    pub(crate) fn schedule(mut self, cron: impl Into<String>) -> Self {
        self.schedule.push(Schedule { cron: cron.into() });
        self
    }

    pub(crate) fn pull_request(
        mut self,
        types: impl IntoIterator<Item = impl Into<String>>,
        branches: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.pull_request = Some(PullRequest {
            types: types.into_iter().map(Into::into).collect(),
            branches: branches.into_iter().map(Into::into).collect(),
        });
        self
    }

    pub(crate) fn pull_request_target(
        mut self,
        types: impl IntoIterator<Item = impl Into<String>>,
        branches: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.pull_request_target = Some(PullRequest {
            types: types.into_iter().map(Into::into).collect(),
            branches: branches.into_iter().map(Into::into).collect(),
        });
        self
    }

    pub(crate) fn issues(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.issues = Some(Issues { types: types.into_iter().map(Into::into).collect() });
        self
    }

    pub(crate) fn release(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.release = Some(Release { types: types.into_iter().map(Into::into).collect() });
        self
    }
}

#[derive(Clone, Serialize)]
struct Schedule {
    cron: String,
}

#[derive(Clone, Serialize)]
struct PullRequest {
    types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Issues {
    types: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Release {
    types: Vec<String>,
}

#[derive(Clone, Serialize)]
struct Concurrency {
    group: String,
    #[serde(rename = "cancel-in-progress")]
    cancel_in_progress: bool,
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Push {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

impl Push {
    pub(crate) fn add_branch(mut self, branch: impl Into<String>) -> Self {
        self.branches.push(branch.into());
        self
    }

    pub(crate) fn add_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(transparent)]
pub(crate) struct Permissions(IndexMap<String, Level>);

impl Permissions {
    pub(crate) fn contents(mut self, level: Level) -> Self {
        self.0.insert("contents".to_string(), level);
        self
    }

    pub(crate) fn issues(mut self, level: Level) -> Self {
        self.0.insert("issues".to_string(), level);
        self
    }

    pub(crate) fn pull_requests(mut self, level: Level) -> Self {
        self.0.insert("pull-requests".to_string(), level);
        self
    }

    pub(crate) fn id_token(mut self, level: Level) -> Self {
        self.0.insert("id-token".to_string(), level);
        self
    }

    pub(crate) fn attestations(mut self, level: Level) -> Self {
        self.0.insert("attestations".to_string(), level);
        self
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Level {
    Read,
    Write,
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Job {
    #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    name: String,
    #[serde(rename = "runs-on")]
    runs_on: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    needs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy: Option<Value>,
    #[serde(skip_serializing_if = "Permissions::is_empty")]
    permissions: Permissions,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    outputs: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    steps: Vec<Step>,
}

impl Job {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            runs_on: "ubuntu-latest".to_string(),
            ..Self::default()
        }
    }

    pub(crate) fn add_step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    pub(crate) fn permissions(mut self, permissions: Permissions) -> Self {
        self.permissions = permissions;
        self
    }

    pub(crate) fn if_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    pub(crate) fn runs_on(mut self, runs_on: impl Into<String>) -> Self {
        self.runs_on = runs_on.into();
        self
    }

    pub(crate) fn needs(mut self, needs: impl Into<String>) -> Self {
        self.needs = Some(needs.into());
        self
    }

    pub(crate) fn strategy(mut self, strategy: Value) -> Self {
        self.strategy = Some(strategy);
        self
    }

    pub(crate) fn output(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.outputs.insert(key.into(), value.into());
        self
    }
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Step {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    uses: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
    #[serde(rename = "with", skip_serializing_if = "IndexMap::is_empty")]
    inputs: IndexMap<String, String>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    env: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(dead_code)]
    value: Option<Value>,
}

impl Step {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Self::default() }
    }

    pub(crate) fn uses(
        mut self,
        owner: impl AsRef<str>,
        action: impl AsRef<str>,
        revision: impl AsRef<str>,
    ) -> Self {
        self.uses = Some(format!(
            "{}/{}@{}",
            owner.as_ref(),
            action.as_ref(),
            revision.as_ref()
        ));
        self
    }

    pub(crate) fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    #[allow(dead_code)]
    pub(crate) fn run(mut self, command: impl Into<String>) -> Self {
        self.run = Some(command.into());
        self
    }

    pub(crate) fn if_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    pub(crate) fn shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
        self
    }

    pub(crate) fn input(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inputs.insert(key.into(), value.into());
        self
    }

    pub(crate) fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}
