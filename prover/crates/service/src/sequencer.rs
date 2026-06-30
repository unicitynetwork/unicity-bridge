use bridge_return_host::s2::{rebuild, RebuiltAccumulator, SettledBatch};

pub fn rebuild_accumulator(
    batches: &[SettledBatch],
) -> bridge_return_host::Result<RebuiltAccumulator> {
    rebuild(batches)
}
