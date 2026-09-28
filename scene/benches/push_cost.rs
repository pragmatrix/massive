//! Push-cost comparison of the two task-scoped change-collection designs, criterion-timed:
//!
//! - `any`:      the [`massive_scene::AnyCollector`] task-local; typed access and the erased
//!   sink handle path.
//! - `collector`: the typed `massive_util::ChangeCollector<SceneChange>` task-local.
//!
//! Each benchmark drains a full batch of `CAPACITY` changes, with
//! `Throughput::Elements(CAPACITY)`, so the reported time per element is the per-push cost with the
//! amortized drain. Both run inside their task-local scope with one `.with` per push.
//!
//! Pushing consumes the change, so each push builds a fresh one from the real id generator. That
//! construction is on the measured path and is identical across both variants, so the comparison
//! holds; the absolute per-element time includes it.
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

/// One transform-update change. Successive acquires from the real per-type generator mimic the id
/// distribution handles produce in practice; building it per push keeps the benchmark on the
/// by-value path (`SceneChange` is moved, not cloned).
fn change() -> SceneChange {
    let id = massive_scene::id_generator::acquire::<massive_geometry::Transform>();
    SceneChange::Transform(massive_scene::Change::Update(
        id,
        transform(id.to_usize() as u32),
    ))
}

fn any_group(c: &mut Criterion) {
    let mut group = c.benchmark_group("task-scope any");
    group.throughput(Throughput::Elements(CAPACITY as u64));
    group.measurement_time(MEASUREMENT);
    group.sample_size(10);

    group.bench_function("typed access: .with per push", |b| {
        b.iter(|| {
            for _ in 0..CAPACITY {
                SCENE.with(|any| any.collect::<SceneChange>(change()));
            }
            SCENE.with(|any| {
                let _ = any.take_all::<SceneChange>();
            });
        })
    });

    group.bench_function("sink(): .with per push", |b| {
        b.iter(|| {
            for _ in 0..CAPACITY {
                SCENE.with(|any| any.sink().send(change()));
            }
            SCENE.with(|any| {
                let _ = any.take_all::<SceneChange>();
            });
        })
    });

    group.finish();
}

fn collector_group(c: &mut Criterion) {
    let mut group = c.benchmark_group("task-scope collector");
    group.throughput(Throughput::Elements(CAPACITY as u64));
    group.measurement_time(MEASUREMENT);
    group.sample_size(10);

    group.bench_function("typed collect: .with per push", |b| {
        b.iter(|| {
            for _ in 0..CAPACITY {
                TYPED.with(|collector| collector.collect(change()));
            }
            TYPED.with(|collector| {
                let _ = collector.take_all();
            });
        })
    });

    group.finish();
}

fn bench(c: &mut Criterion) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime")
        .block_on(async {
            SCENE
                .scope(AnyCollector::for_type::<SceneChange>(), async {
                    any_group(c);
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
                        collector_group(c);
                    },
                )
                .await;
        });
}

criterion_group!(push_cost, bench);
criterion_main!(push_cost);
