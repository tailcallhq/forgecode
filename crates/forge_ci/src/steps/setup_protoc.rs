use crate::workflow_model::Step;

/// Creates a step to setup the Protobuf compiler.
///
/// This step is reusable across all CI workflows that need protobuf
/// compilation.
pub(crate) fn setup_protoc() -> Step {
    Step::new("Setup Protobuf Compiler")
        .uses(
            "arduino",
            "setup-protoc",
            "c65c819552d16ad3c9b72d9dfd5ba5237b9c906b",
        )
        .input("repo-token", "${{ secrets.GITHUB_TOKEN }}")
}
