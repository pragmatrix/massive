//! Push-cost comparison of the two task-scoped change-collection designs, criterion-timed:
//!
//! - `any`:      the [`massive_scene::AnyCollector`] task-local; typed access and the erased
//!   sink handle path.
//! - `collector`: the typed `massive_util::ChangeCollector<SceneChange>` task-local.
//!
//! Each benchmark pushes a full batch, then drains, with `Throughput::Elements(CAPACITY)`, so
//! the reported time per element is the per-push cost with the amortized drain. Both run inside
//! their task-local scope with one `.with` per push.
//!
//! The task-locals only need their scope alive while criterion runs the closures; the scopes
//! wrap the group drivers inside `block_on` on a current-thread runtime (task-local visibility
//! is a thread property, so any per-push `.with` outside a scope panics).
//!
//! Run with: `cargo bench -p massive-scene --bench push_cost`.

use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use massive_scene::{AnyCollector, SceneChange};
use tokio::task_local;

const CAPACITY: usize = 100_000;
/// Per-variant measurement time; criterion adds its own warmup and sampling.
const MEASUREMENT: Duration = Duration::from_secs(1);

task_local! {
    static SCENE: AnyCollector;
    static TYPED: massive_util::ChangeCollector<SceneChange>;
}

fn transform(id: u32) -> massive_geometry::Transform {
    massive_geometry::Transform::from_translation(massive_geometry::Vector3::new(
        id as f64, 0.0, 0.0,
    ))
}

/// One full round of changes: successive acquires from the real per-type generator mimic the id
/// distribution handles produce in practice.
fn make_changes() -> Vec<SceneChange> {
    (0..CAPACITY)
        .map(|_| {
            let id = massive_scene::id_generator::acquire::<massive_geometry::Transform>();
            SceneChange::Transform(massive_scene::Change::Update(
                id,
                transform(id.to_usize() as u32),
            ))
        })
        .collect()
}

// AnyCollector variant (erased write path). Both closures run under the SCENE scope installed
// by the driver below, so the task-local access is the path the sink takes.
fn any_group(c: &mut Criterion, changes: &[SceneChange]) {
    let mut group = c.benchmark_group("task-scope any");
    group.throughput(Throughput::Elements(CAPACITY as u64));
    group.measurement_time(MEASUREMENT);
    group.sample_size(10);

    group.bench_function("typed access: .with per push", |b| {
        b.iter(|| {
            for change in changes {
                SCENE.with(|any| any.collect::<SceneChange>(change.clone()));
            }
            SCENE.with(|any| {
                let _ = any.take_all::<SceneChange>();
            });
        })
    });

    group.bench_function("sink(): .with per push", |b| {
        b.iter(|| {
            for change in changes {
                SCENE.with(|any| any.sink().send(change.clone()));
            }
            SCENE.with(|any| {
                let _ = any.take_all::<SceneChange>();
            });
        })
    });

    group.finish();
}

// Typed collector variant.
fn collector_group(c: &mut Criterion, changes: &[SceneChange]) {
    let mut group = c.benchmark_group("task-scope collector");
    group.throughput(Throughput::Elements(CAPACITY as u64));
    group.measurement_time(MEASUREMENT);
    group.sample_size(10);

    group.bench_function("typed collect: .with per push", |b| {
        b.iter(|| {
            for change in changes {
                TYPED.with(|collector| collector.collect(change.clone()));
            }
            TYPED.with(|collector| {
                let _ = collector.take_all();
            });
        })
    });

    group.finish();
}

fn bench(c: &mut Criterion) {
    let changes = make_changes();

    // Task-locals must be installed while criterion runs the closures; the scopes wrap the
    // whole group driver.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime")
        .block_on(async {
            SCENE
                .scope(AnyCollector::for_type::<SceneChange>(), async {
                    any_group(c, &changes);
                })
                .await;
        });

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime")
        .block_on(async {
            TYPED
                .scope(
                    massive_util::ChangeCollector::<SceneChange>::default(),
                    async {
                        collector_group(c, &changes);
                    },
                )
                .await;
        });
}

criterion_group!(push_cost, bench);
criterion_main!(push_cost);
