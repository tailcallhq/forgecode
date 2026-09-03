use crate::workflow_model::Job;

use crate::jobs::release_build_job::release_build_job;

/// Create a build job for drafts
pub fn create_build_release_job_for_publishing() -> Job {
    release_build_job("${{ github.event.release.tag_name }}", true)
}
