use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use forge_app::ConversationService;
use forge_domain::{Conversation, ConversationId, WorkspaceHash};
use forge_repo::{ConversationRepositoryImpl, DatabasePool, PoolConfig};
use forge_services::ForgeConversationService;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio::task::JoinSet;

struct BenchmarkFixture {
    _temp_dir: TempDir,
    service: Arc<ForgeConversationService<ConversationRepositoryImpl>>,
}

impl BenchmarkFixture {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let database_path = temp_dir.path().join("bench.sqlite");
        let mut pool_config = PoolConfig::new(database_path);
        pool_config.max_size = 5;
        pool_config.min_idle = Some(1);
        pool_config.connection_timeout = Duration::from_secs(5);
        let pool = Arc::new(DatabasePool::try_from(pool_config).unwrap());
        let repository = Arc::new(ConversationRepositoryImpl::new(pool, WorkspaceHash::new(0)));
        let service = Arc::new(ForgeConversationService::new(repository));

        Self {
            _temp_dir: temp_dir,
            service,
        }
    }
}

async fn run_parallel_same_conversation_writes(
    service: Arc<ForgeConversationService<ConversationRepositoryImpl>>,
    tasks: usize,
    writes_per_task: usize,
) {
    let conversation_id = ConversationId::generate();
    let mut join_set = JoinSet::new();

    for task_index in 0..tasks {
        let service = service.clone();
        join_set.spawn(async move {
            for write_index in 0..writes_per_task {
                let title = format!("task-{task_index}-write-{write_index}");
                let conversation = Conversation::new(conversation_id).title(title);
                service.upsert_conversation(conversation).await.unwrap();
            }
        });
    }

    while let Some(result) = join_set.join_next().await {
        result.unwrap();
    }
}

fn bench_parallel_same_conversation_writes(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("conversation_persistence");

    for (tasks, writes_per_task) in [(4usize, 10usize), (8, 10), (16, 10)] {
        let fixture = BenchmarkFixture::new();
        group.throughput(Throughput::Elements((tasks * writes_per_task) as u64));
        group.bench_function(
            BenchmarkId::from_parameter(format!("same-id-{tasks}x{writes_per_task}")),
            |b| {
                b.to_async(&runtime).iter(|| async {
                    run_parallel_same_conversation_writes(
                        fixture.service.clone(),
                        tasks,
                        writes_per_task,
                    )
                    .await;
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_parallel_same_conversation_writes);
criterion_main!(benches);
