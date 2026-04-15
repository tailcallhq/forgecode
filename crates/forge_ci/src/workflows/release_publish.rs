use gh_workflow::generate::Generate;
use gh_workflow::*;

use crate::jobs::{ReleaseBuilderJob, release_bench_job, release_homebrew_job, release_npm_job};

/// Generate npm release workflow
pub fn release_publish() {
    let release_build_job = ReleaseBuilderJob::new("${{ github.event.release.tag_name }}")
        .release_id("${{ github.event.release.id }}");
    let npm_release_job = release_npm_job().add_needs("build_release");
    let homebrew_release_job = release_homebrew_job().add_needs("build_release");
    let bench_job = release_bench_job().add_needs("build_release");

    let npm_workflow = Workflow::default()
        .name("Multi Channel Release")
        .on(Event {
            release: Some(Release { types: vec![ReleaseType::Published] }),
            ..Event::default()
        })
        .permissions(
            Permissions::default()
                .contents(Level::Write)
                .pull_requests(Level::Write),
        )
        .add_job("build_release", release_build_job.into_job())
        .add_job("npm_release", npm_release_job)
        .add_job("homebrew_release", homebrew_release_job)
        .add_job("bench", bench_job);

    Generate::new(npm_workflow)
        .name("release.yml")
        .generate()
        .unwrap();
}
