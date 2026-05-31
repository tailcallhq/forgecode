use std::sync::Arc;

use forge_app::TodoService;
use forge_domain::{ConversationId, Todo, TodoRepository};
use uuid::Uuid;

/// Implementation of TodoService
pub struct ForgeTodoService<F> {
    infra: Arc<F>,
}

impl<F> ForgeTodoService<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: TodoRepository> TodoService for ForgeTodoService<F> {
    async fn update_todos(
        &self,
        conversation_id: &ConversationId,
        mut todos: Vec<Todo>,
    ) -> anyhow::Result<Vec<Todo>> {
        // Load existing todos (for future use in merging/updating)
        let _existing_todos = self.infra.get_todos(conversation_id).await?;

        // Process each todo
        for todo in &mut todos {
            // Validate the todo
            todo.validate()?;

            // If todo doesn't have an ID, generate one
            if todo.id.is_empty() {
                todo.id = Uuid::new_v4().to_string();
            }
        }

        // Validate unique IDs
        let ids: Vec<&str> = todos.iter().map(|t| t.id.as_str()).collect();
        let unique_ids: std::collections::HashSet<&str> = ids.iter().copied().collect();
        if ids.len() != unique_ids.len() {
            anyhow::bail!("Duplicate todo IDs found in the request");
        }

        // Save updated todos
        self.infra
            .save_todos(conversation_id, todos.clone())
            .await?;

        Ok(todos)
    }

    async fn get_todos(&self, conversation_id: &ConversationId) -> anyhow::Result<Vec<Todo>> {
        self.infra.get_todos(conversation_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use forge_domain::TodoStatus;
    use pretty_assertions::assert_eq;

    use super::*;

    struct MockInfra {
        todos: Arc<Mutex<HashMap<String, Vec<Todo>>>>,
    }

    impl MockInfra {
        fn new() -> Self {
            Self { todos: Arc::new(Mutex::new(HashMap::new())) }
        }
    }

    #[async_trait::async_trait]
    impl forge_domain::TodoRepository for MockInfra {
        async fn save_todos(
            &self,
            conversation_id: &ConversationId,
            todos: Vec<Todo>,
        ) -> anyhow::Result<()> {
            self.todos
                .lock()
                .unwrap()
                .insert(conversation_id.to_string(), todos);
            Ok(())
        }

        async fn get_todos(&self, conversation_id: &ConversationId) -> anyhow::Result<Vec<Todo>> {
            Ok(self
                .todos
                .lock()
                .unwrap()
                .get(&conversation_id.to_string())
                .cloned()
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn test_update_todos_generates_ids() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        let todos = vec![Todo::new("Task 1").id(String::new())]; // Empty ID

        let result = service.update_todos(&conversation_id, todos).await.unwrap();

        assert_eq!(result.len(), 1);
        assert!(!result[0].id.is_empty());
        assert_eq!(result[0].content, "Task 1");
    }

    #[tokio::test]
    async fn test_update_todos_merges_by_id() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        // Create initial todo
        let todo1 = Todo::new("Task 1").id("todo-1".to_string());

        service
            .update_todos(&conversation_id, vec![todo1])
            .await
            .unwrap();

        // Update the same todo
        let todo2 = Todo::new("Task 1 Updated")
            .id("todo-1".to_string())
            .status(TodoStatus::Completed);

        let result = service
            .update_todos(&conversation_id, vec![todo2])
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "todo-1");
        assert_eq!(result[0].content, "Task 1 Updated");
        assert_eq!(result[0].status, TodoStatus::Completed);
    }

    #[tokio::test]
    async fn test_update_todos_validates_content() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        let todos = vec![Todo::new("").content("")]; // Empty content

        let result = service.update_todos(&conversation_id, todos).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_update_todos_rejects_duplicate_ids() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        let todos = vec![
            Todo::new("Task 1").id("same-id".to_string()),
            Todo::new("Task 2").id("same-id".to_string()),
        ];

        let result = service.update_todos(&conversation_id, todos).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate todo IDs")
        );
    }

    #[tokio::test]
    async fn test_get_todos_empty() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        let result = service.get_todos(&conversation_id).await.unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_update_and_get_todos() {
        let infra = Arc::new(MockInfra::new());
        let service = ForgeTodoService::new(infra);
        let conversation_id = ConversationId::generate();

        let todos = vec![Todo::new("Task 1"), Todo::new("Task 2")];

        service
            .update_todos(&conversation_id, todos.clone())
            .await
            .unwrap();

        let retrieved = service.get_todos(&conversation_id).await.unwrap();

        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].content, "Task 1");
        assert_eq!(retrieved[1].content, "Task 2");
    }
}
