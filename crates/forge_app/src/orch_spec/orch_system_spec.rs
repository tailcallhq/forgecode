use forge_domain::{ChatCompletionMessage, CommandOutput, Content, FinishReason};
use insta::assert_snapshot;

use crate::ShellOutput;
use crate::orch_spec::orch_runner::TestContext;

#[tokio::test]
async fn test_system_prompt() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Sure")).finish_reason(FinishReason::Stop),
    ]);

    ctx.run("This is a test").await.unwrap();
    let system_messages = ctx.output.system_messages().unwrap().join("\n\n");
    assert_snapshot!(system_messages);
}

#[tokio::test]
async fn test_system_prompt_tool_supported() {
    let mut ctx = TestContext::default()
        .files(vec![
            forge_domain::File { path: "/users/john/foo.txt".to_string(), is_dir: false },
            forge_domain::File { path: "/users/jason/bar.txt".to_string(), is_dir: false },
        ])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Sure"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("This is a test").await.unwrap();

    let system_messages = ctx.output.system_messages().unwrap().join("\n\n");
    assert_snapshot!(system_messages);
}

#[tokio::test]
async fn test_system_prompt_with_extensions() {
    let shell_output = ShellOutput {
        output: CommandOutput {
            stdout: include_str!("../fixtures/git_ls_files_mixed.txt").to_string(),
            stderr: String::new(),
            command: "git ls-files".to_string(),
            exit_code: Some(0),
        },
        shell: "/bin/bash".to_string(),
        description: None,
    };

    let mut ctx = TestContext::default()
        .mock_shell_outputs(vec![shell_output])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Sure"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("This is a test").await.unwrap();

    let system_messages = ctx.output.system_messages().unwrap().join("\n\n");
    assert_snapshot!(system_messages);
}

#[tokio::test]
async fn test_system_prompt_with_extensions_truncated() {
    // Create 20 different file extensions to test truncation
    let mut files = Vec::new();
    for i in 1..=20 {
        // Each extension gets 21-i files (so ext1 has most, ext20 has least)
        for j in 0..(21 - i) {
            files.push(format!("file{}.ext{}", j, i));
        }
    }
    let stdout = files.join("\n");

    let shell_output = ShellOutput {
        output: CommandOutput {
            stdout,
            stderr: String::new(),
            command: "git ls-files".to_string(),
            exit_code: Some(0),
        },
        shell: "/bin/bash".to_string(),
        description: None,
    };

    let mut ctx = TestContext::default()
        .mock_shell_outputs(vec![shell_output])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Sure"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("This is a test").await.unwrap();

    let system_messages = ctx.output.system_messages().unwrap().join("\n\n");
    assert_snapshot!(system_messages);
}
