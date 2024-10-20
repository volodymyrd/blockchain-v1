use chain_model::block::Approval;
use chain_model::clock::Clock;
use chain_model::crypto::CryptoHash;
use chain_model::types::{BlockHeight, BlockHeightDelta};
use std::time::{Duration, Instant};

/// Have that many iterations in the timer instead of `loop` to prevent potential bugs from blocking
/// the node
const MAX_TIMER_ITERS: usize = 20;

struct DoomslugTimer {
    started: Instant,
    last_endorsement_sent: Instant,
    height: BlockHeight,
    endorsement_delay: Duration,
    min_delay: Duration,
    delay_step: Duration,
    max_delay: Duration,
}

impl DoomslugTimer {
    /// Computes the delay to sleep given the number of heights from the last final block
    /// This is what `T` represents in the paper.
    ///
    /// # Arguments
    /// * `n` - number of heights since the last block with doomslug finality
    ///
    /// # Returns
    /// Duration to sleep
    pub fn get_delay(&self, n: BlockHeightDelta) -> Duration {
        let n32 = u32::try_from(n).unwrap_or(u32::MAX);
        std::cmp::min(
            self.max_delay,
            self.min_delay + self.delay_step * n32.saturating_sub(2),
        )
    }
}

struct DoomslugTip {
    block_hash: CryptoHash,
    height: BlockHeight,
}

struct Doomslug {
    clock: Clock,
    /// Largest target height for which we issued an approval
    largest_target_height: BlockHeight,
    /// Largest height for which we saw a block containing 1/2 endorsements in it
    largest_final_height: BlockHeight,
    /// Information Doomslug tracks about the chain tip
    tip: DoomslugTip,
    /// Whether an endorsement (or in general an approval) was sent since updating the tip
    endorsement_pending: bool,
    /// Information to track the timer.
    timer: DoomslugTimer,
}

impl Doomslug {
    fn new(
        clock: Clock,
        largest_target_height: BlockHeight,
        endorsement_delay: Duration,
        min_delay: Duration,
        delay_step: Duration,
        max_delay: Duration,
    ) -> Self {
        let now = clock.now();
        Self {
            clock,
            largest_target_height,
            largest_final_height: 0,
            tip: DoomslugTip {
                block_hash: CryptoHash::default(),
                height: 0,
            },
            endorsement_pending: false,
            timer: DoomslugTimer {
                started: now,
                last_endorsement_sent: now,
                height: 0,
                endorsement_delay,
                min_delay,
                delay_step,
                max_delay,
            },
        }
    }

    /// Updates the current tip of the chain. Restarts the timer accordingly.
    ///
    /// # Arguments
    /// * `block_hash`     - the hash of the new tip
    /// * `height`         - the height of the tip
    /// * `last_ds_final_height` - last height at which a block in this chain has doomslug finality
    pub fn set_tip(
        &mut self,
        block_hash: CryptoHash,
        height: BlockHeight,
        last_final_height: BlockHeight,
    ) {
        self.tip = DoomslugTip { block_hash, height };

        self.largest_final_height = last_final_height;
        self.timer.height = height + 1;
        self.timer.started = self.clock.now();

        self.endorsement_pending = true;
    }

    fn create_approval(
        &self,
        target_height: BlockHeight,
        //signer: &Option<Arc<ValidatorSigner>>,
    ) -> Option<Approval> {
        //signer.as_ref().map(|signer| {
        Some(Approval::new(
            self.tip.block_hash,
            self.tip.height,
            target_height,
        ))
        //})
    }

    /// Returns a vector of approvals that need to be sent to other block producers as a result
    /// of processing the timers.
    fn process_timer(&mut self) -> Vec<Approval> {
        let now = self.clock.now();
        let mut approvals = vec![];
        for _ in 0..MAX_TIMER_ITERS {
            let skip_delay = self
                .timer
                .get_delay(self.timer.height.saturating_sub(self.largest_final_height));

            // The `endorsement_delay` is time to send approval to the block producer at `timer.height`,
            // while the `skip_delay` is the time before sending the approval to BP of `timer_height + 1`,
            // so it makes sense for them to be at least 2x apart
            debug_assert!(skip_delay >= 2 * self.timer.endorsement_delay);

            let tip_height = self.tip.height;
            if self.endorsement_pending
                && now >= self.timer.last_endorsement_sent + self.timer.endorsement_delay
            {
                if tip_height >= self.largest_target_height {
                    self.largest_target_height = tip_height + 1;

                    if let Some(approval) = self.create_approval(tip_height + 1) {
                        approvals.push(approval);
                    }
                }

                self.timer.last_endorsement_sent = now;
                self.endorsement_pending = false;
            }
        }
        approvals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_model::block::ApprovalInner;
    use chain_model::clock::Clock;
    use chain_model::crypto::hash;

    #[test]
    fn test_endorsements_and_skips_basic() {
        let mut clock = Clock::fake_new();
        let mut ds = Doomslug::new(
            clock.clone(),
            0,
            Duration::from_millis(400),
            Duration::from_millis(1000),
            Duration::from_millis(100),
            Duration::from_millis(3000),
        );

        // Set a new tip, must produce an endorsement
        ds.set_tip(hash(&[123]), 1, 1);
        clock.advance(Duration::from_millis(399));
        assert_eq!(ds.process_timer().len(), 0);
        clock.advance(Duration::from_millis(1));
        let approval = ds.process_timer().into_iter().nth(0).unwrap();
        assert_eq!(approval.inner, ApprovalInner::Endorsement(hash(&[123])));
        assert_eq!(approval.target_height, 2);
    }
}
