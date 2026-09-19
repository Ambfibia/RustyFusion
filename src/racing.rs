use std::{
    collections::HashSet,
    time::{Duration, SystemTime},
};

use serde::Deserialize;

use crate::{
    enums::RacingMode,
    error::{FFError, FFResult, Severity},
};

/// Number of rank tiers an Infected Zone race is scored against.
pub const NUM_RACE_RANKS: usize = 5;

/// Static, per-Infected-Zone race configuration.
///
/// The XDT only knows the EP id and the score cap; everything else that the
/// scoring formula needs (time limit, pod count, curve factors) comes from
/// `drops.json`, same as OpenFusion.
#[derive(Debug, Clone, Deserialize)]
pub struct RaceData {
    #[serde(rename = "EPID")]
    pub ep_id: i32,
    #[serde(rename = "RankScores")]
    pub rank_scores: Vec<i32>,
    #[serde(rename = "Rewards")]
    pub rank_rewards: Vec<i32>,
    #[serde(rename = "TimeLimit")]
    pub time_limit_s: i32,
    #[serde(rename = "ScoreCap")]
    pub score_cap: i32,
    #[serde(rename = "TotalPods")]
    pub total_pods: i32,
    #[serde(rename = "ScaleFactor")]
    pub scale_factor: f64,
    #[serde(rename = "PodFactor")]
    pub pod_factor: f64,
    #[serde(rename = "TimeFactor")]
    pub time_factor: f64,
    #[serde(rename = "EPName", default)]
    pub ep_name: String,
}
impl RaceData {
    pub fn validate(&self) -> Result<(), String> {
        if self.rank_scores.len() != NUM_RACE_RANKS
            || self.rank_rewards.len() != self.rank_scores.len()
        {
            return Err(format!(
                "Race in EP {} doesn't have exactly {} score/reward pairs",
                self.ep_id, NUM_RACE_RANKS
            ));
        }
        if self.total_pods <= 0 || self.time_limit_s <= 0 {
            return Err(format!(
                "Race in EP {} has a non-positive pod count or time limit",
                self.ep_id
            ));
        }
        Ok(())
    }

    pub fn get_time_limit(&self) -> Duration {
        Duration::from_secs(self.time_limit_s as u64)
    }

    /// Scores a run. Returns `(score, fusion_matter)`.
    ///
    /// This is the same exponential curve OpenFusion uses, so records stay
    /// comparable between the two servers.
    pub fn score_run(&self, num_pods: usize, elapsed: Duration, cap_score: bool) -> (i32, u32) {
        let num_pods = num_pods as f64;
        let elapsed_s = elapsed.as_secs() as f64;
        let max_pods = self.total_pods as f64;
        let max_time = self.time_limit_s as f64;

        let exponent = (self.pod_factor * num_pods) / max_pods
            - (self.time_factor * elapsed_s) / max_time
            + self.scale_factor;
        let mut score = exponent.exp() as i32;
        if cap_score && score > self.score_cap {
            score = self.score_cap;
        }

        let fm = (1.0 + (self.scale_factor - 1.0).exp() * self.pod_factor * num_pods) / max_pods;
        (score, fm.max(0.0) as u32)
    }

    /// Rank tier (1-based) for a given score. Rank 1 is the best.
    pub fn get_rank(&self, score: i32) -> usize {
        let max_rank_idx = self.rank_scores.len() - 1;
        let mut rank = 0;
        while rank < max_rank_idx && self.rank_scores[rank] > score {
            rank += 1;
        }
        rank + 1
    }

    /// The C.R.A.T.E. id awarded for a given 1-based rank, if any.
    pub fn get_reward_crate_id(&self, rank: usize) -> Option<i16> {
        let crate_id = *self.rank_rewards.get(rank - 1)?;
        if crate_id == 0 {
            None
        } else {
            Some(crate_id as i16)
        }
    }
}

/// A race result as persisted in the `RaceResults` table.
#[derive(Debug, Clone, Copy)]
pub struct RaceResult {
    pub ep_id: i32,
    pub pc_uid: i64,
    pub score: i32,
    pub num_pods: i32,
    pub time_s: i32,
    pub timestamp: u32,
}
impl RaceResult {
    pub fn blank(ep_id: i32, pc_uid: i64) -> Self {
        Self {
            ep_id,
            pc_uid,
            score: 0,
            num_pods: 0,
            time_s: 0,
            timestamp: 0,
        }
    }
}

/// A race that a player is currently running.
#[derive(Debug, Clone)]
pub struct RaceState {
    pub ep_id: i32,
    pub map_num: u32,
    pub mode: RacingMode,
    pub start_npc_id: i32,
    pub ticket_slot: Option<usize>,
    pub start_time: SystemTime,
    collected_rings: HashSet<i32>,
}
impl RaceState {
    pub fn new(
        ep_id: i32,
        map_num: u32,
        mode: RacingMode,
        start_npc_id: i32,
        ticket_slot: Option<usize>,
    ) -> Self {
        Self {
            ep_id,
            map_num,
            mode,
            start_npc_id,
            ticket_slot,
            start_time: SystemTime::now(),
            collected_rings: HashSet::new(),
        }
    }

    /// Records a ring pickup. Returns the new ring count, or an error if the
    /// client tried to bank the same ring twice.
    pub fn collect_ring(&mut self, ring_id: i32) -> FFResult<usize> {
        if !self.collected_rings.insert(ring_id) {
            return Err(FFError::build(
                Severity::Warning,
                format!("Ring {} was already collected in this race", ring_id),
            ));
        }
        Ok(self.collected_rings.len())
    }

    pub fn get_num_rings(&self) -> usize {
        self.collected_rings.len()
    }

    pub fn get_elapsed(&self) -> Duration {
        self.start_time.elapsed().unwrap_or(Duration::ZERO)
    }
}
