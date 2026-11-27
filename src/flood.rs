//! Pure time-decayed primitives for conversation flood detectors.
//!
//! These types deliberately do not choose policy thresholds or take actions.
//! Callers provide an event-time timeline in microseconds. Samples must be
//! submitted in nondecreasing event-time order; consumers that accept late
//! Telegram updates must reorder them or mark their evidence incomplete
//! before updating these primitives.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
};

/// A timestamp on the caller's monotonic event-time timeline, in microseconds.
pub type EventTimeMicros = u64;

/// Invalid input to a time-decayed statistic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EwmaError {
    /// The decay time constant must be greater than zero.
    ZeroTimeConstant,
    /// An event-rate observation must contain at least one event.
    ZeroEventWeight,
    /// A weighted-mean observation must have finite value and positive finite weight.
    InvalidSample,
    /// Samples were submitted earlier than the last accepted event time.
    OutOfOrder {
        last_accepted: EventTimeMicros,
        received: EventTimeMicros,
    },
    /// An update would produce a non-finite internal value.
    ArithmeticOverflow,
}

impl fmt::Display for EwmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimeConstant => f.write_str("EWMA time constant must be positive"),
            Self::ZeroEventWeight => f.write_str("EWMA event weight must be positive"),
            Self::InvalidSample => {
                f.write_str("EWMA sample must be finite and its weight positive and finite")
            }
            Self::OutOfOrder {
                last_accepted,
                received,
            } => write!(
                f,
                "EWMA event time {received} precedes last accepted time {last_accepted}"
            ),
            Self::ArithmeticOverflow => f.write_str("EWMA update produced a non-finite value"),
        }
    }
}

impl std::error::Error for EwmaError {}

/// An event rate with exponential decay.
///
/// For time constant `tau`, each observation of weight `w` contributes
/// `w / tau_seconds` events per second, and prior state decays by
/// `exp(-delta / tau)`. This is a rate, not a hard-window count; use an exact
/// window or token bucket for hard limits.
#[derive(Clone, Debug, PartialEq)]
pub struct EwmaRate {
    tau_micros: u64,
    last_at: Option<EventTimeMicros>,
    rate_per_second: f64,
    accepted_events: u64,
}

impl EwmaRate {
    /// Creates an empty rate with a positive time constant in microseconds.
    pub fn new(tau_micros: u64) -> Result<Self, EwmaError> {
        if tau_micros == 0 {
            return Err(EwmaError::ZeroTimeConstant);
        }
        Ok(Self {
            tau_micros,
            last_at: None,
            rate_per_second: 0.0,
            accepted_events: 0,
        })
    }

    /// Adds `weight` logical events at `at`.
    ///
    /// Invalid or out-of-order input leaves the statistic unchanged.
    pub fn observe(&mut self, at: EventTimeMicros, weight: u32) -> Result<(), EwmaError> {
        if weight == 0 {
            return Err(EwmaError::ZeroEventWeight);
        }
        self.validate_time(at)?;

        let decayed = self.rate_at_unchecked(at);
        let contribution = f64::from(weight) * 1_000_000.0 / self.tau_micros as f64;
        let next = decayed + contribution;
        if !next.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }

        self.rate_per_second = next;
        self.last_at = Some(at);
        self.accepted_events = self.accepted_events.saturating_add(u64::from(weight));
        Ok(())
    }

    /// Returns the decayed events-per-second estimate at `at`.
    ///
    /// Reading at a future time does not mutate the statistic. A timestamp
    /// before the last accepted event is rejected rather than clamped.
    pub fn rate_at(&self, at: EventTimeMicros) -> Result<Option<f64>, EwmaError> {
        let Some(last_at) = self.last_at else {
            return Ok(None);
        };
        if at < last_at {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }
        Ok(Some(self.rate_at_unchecked(at)))
    }

    /// Number of logical events accepted, including weights.
    pub fn accepted_events(&self) -> u64 {
        self.accepted_events
    }

    /// The configured exponential time constant, in microseconds.
    pub fn tau_micros(&self) -> u64 {
        self.tau_micros
    }

    fn validate_time(&self, at: EventTimeMicros) -> Result<(), EwmaError> {
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }
        Ok(())
    }

    fn rate_at_unchecked(&self, at: EventTimeMicros) -> f64 {
        let Some(last_at) = self.last_at else {
            return 0.0;
        };
        let elapsed = at - last_at;
        self.rate_per_second * decay(elapsed, self.tau_micros)
    }
}

/// A time-decayed weighted mean for contextual features.
///
/// The mean is `decayed_weighted_sum / decayed_weight`. Callers choose the
/// minimum effective weight needed before exposing a value; before that, the
/// result is `None` rather than a fabricated zero.
#[derive(Clone, Debug, PartialEq)]
pub struct DecayedMean {
    tau_micros: u64,
    minimum_effective_weight: f64,
    last_at: Option<EventTimeMicros>,
    weighted_sum: f64,
    effective_weight: f64,
    accepted_samples: u64,
}

impl DecayedMean {
    /// Creates a decayed mean with a positive time constant and warmup weight.
    pub fn new(tau_micros: u64, minimum_effective_weight: f64) -> Result<Self, EwmaError> {
        if tau_micros == 0 {
            return Err(EwmaError::ZeroTimeConstant);
        }
        if !minimum_effective_weight.is_finite() || minimum_effective_weight <= 0.0 {
            return Err(EwmaError::InvalidSample);
        }
        Ok(Self {
            tau_micros,
            minimum_effective_weight,
            last_at: None,
            weighted_sum: 0.0,
            effective_weight: 0.0,
            accepted_samples: 0,
        })
    }

    /// Adds one finite value with positive finite weight at event time `at`.
    ///
    /// Invalid, overflowing, or out-of-order input leaves state unchanged.
    pub fn observe(
        &mut self,
        at: EventTimeMicros,
        value: f64,
        weight: f64,
    ) -> Result<(), EwmaError> {
        if !value.is_finite() || !weight.is_finite() || weight <= 0.0 {
            return Err(EwmaError::InvalidSample);
        }
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }

        let factor = self
            .last_at
            .map_or(1.0, |last_at| decay(at - last_at, self.tau_micros));
        let next_sum = self.weighted_sum * factor + value * weight;
        let next_weight = self.effective_weight * factor + weight;
        if !next_sum.is_finite() || !next_weight.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }

        self.weighted_sum = next_sum;
        self.effective_weight = next_weight;
        self.last_at = Some(at);
        self.accepted_samples = self.accepted_samples.saturating_add(1);
        Ok(())
    }

    /// Returns the mean at `at` once enough decayed sample weight remains.
    pub fn mean_at(&self, at: EventTimeMicros) -> Result<Option<f64>, EwmaError> {
        let Some(last_at) = self.last_at else {
            return Ok(None);
        };
        if at < last_at {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }

        let factor = decay(at - last_at, self.tau_micros);
        let effective_weight = self.effective_weight * factor;
        if effective_weight < self.minimum_effective_weight {
            return Ok(None);
        }
        let mean = (self.weighted_sum * factor) / effective_weight;
        if !mean.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }
        Ok(Some(mean))
    }

    /// Number of accepted observations before decay.
    pub fn accepted_samples(&self) -> u64 {
        self.accepted_samples
    }

    /// The current effective sample weight at the most recent event time.
    pub fn effective_weight(&self) -> f64 {
        self.effective_weight
    }
}

/// Errors from the exact sliding window used for hard limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowError {
    ZeroWindow,
    ZeroCapacity,
    ZeroEventWeight,
    OutOfOrder {
        last_accepted: EventTimeMicros,
        received: EventTimeMicros,
    },
    CapacityExceeded,
    CountOverflow,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWindow => f.write_str("sliding-window duration must be positive"),
            Self::ZeroCapacity => f.write_str("sliding-window capacity must be positive"),
            Self::ZeroEventWeight => f.write_str("sliding-window event weight must be positive"),
            Self::OutOfOrder {
                last_accepted,
                received,
            } => write!(
                f,
                "window event time {received} precedes last accepted time {last_accepted}"
            ),
            Self::CapacityExceeded => f.write_str("sliding-window capacity was exceeded"),
            Self::CountOverflow => f.write_str("sliding-window event count overflowed"),
        }
    }
}

impl std::error::Error for WindowError {}

/// Exact weighted event count for `(now - window, now]`.
///
/// Memory is bounded by `capacity`. When an event cannot be retained, the
/// window becomes explicitly incomplete until that dropped event must have
/// aged out. [`count_at`](Self::count_at) then returns `None`, never a
/// deceptively low count. Samples must be submitted in nondecreasing time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactWindow {
    window_micros: u64,
    capacity: usize,
    last_at: Option<EventTimeMicros>,
    events: VecDeque<(EventTimeMicros, u32)>,
    retained_weight: u64,
    incomplete_until: Option<EventTimeMicros>,
    incomplete_forever: bool,
}

impl ExactWindow {
    /// Creates a bounded exact window.
    pub fn new(window_micros: u64, capacity: usize) -> Result<Self, WindowError> {
        if window_micros == 0 {
            return Err(WindowError::ZeroWindow);
        }
        if capacity == 0 {
            return Err(WindowError::ZeroCapacity);
        }
        Ok(Self {
            window_micros,
            capacity,
            last_at: None,
            events: VecDeque::with_capacity(capacity.min(4096)),
            retained_weight: 0,
            incomplete_until: None,
            incomplete_forever: false,
        })
    }

    /// Retains one event with positive `weight` at `at`.
    ///
    /// Capacity overflow is observable and taints the window until the lost
    /// event expires. The event is not retained and the error must not be
    /// interpreted as a zero count.
    pub fn observe(
        &mut self,
        at: EventTimeMicros,
        weight: u32,
    ) -> Result<Option<u64>, WindowError> {
        if weight == 0 {
            return Err(WindowError::ZeroEventWeight);
        }
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(WindowError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }

        let expired_before_or_at = at.checked_sub(self.window_micros);
        let expired_count = self
            .events
            .iter()
            .take_while(|(event_at, _)| {
                expired_before_or_at.is_some_and(|cutoff| *event_at <= cutoff)
            })
            .count();
        let expired_weight = self
            .events
            .iter()
            .take(expired_count)
            .map(|(_, old_weight)| u64::from(*old_weight))
            .sum::<u64>();
        let remaining_len = self.events.len() - expired_count;
        let remaining_weight = self.retained_weight - expired_weight;
        if remaining_len == self.capacity {
            match at.checked_add(self.window_micros) {
                Some(expiry) => {
                    self.incomplete_until = Some(
                        self.incomplete_until
                            .map_or(expiry, |previous| previous.max(expiry)),
                    );
                }
                None => self.incomplete_forever = true,
            }
            for _ in 0..expired_count {
                self.events.pop_front();
            }
            self.retained_weight = remaining_weight;
            self.last_at = Some(at);
            return Err(WindowError::CapacityExceeded);
        }

        let Some(next_weight) = remaining_weight.checked_add(u64::from(weight)) else {
            return Err(WindowError::CountOverflow);
        };
        for _ in 0..expired_count {
            self.events.pop_front();
        }
        self.events.push_back((at, weight));
        self.retained_weight = next_weight;
        self.last_at = Some(at);
        self.count_at(at)
    }

    /// Returns the exact count, or `None` while a dropped event could still
    /// fall inside the requested window.
    pub fn count_at(&self, at: EventTimeMicros) -> Result<Option<u64>, WindowError> {
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(WindowError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }
        if self.incomplete_forever || self.incomplete_until.is_some_and(|until| at < until) {
            return Ok(None);
        }

        let Some(cutoff) = at.checked_sub(self.window_micros) else {
            return Ok(Some(self.retained_weight));
        };
        let expired = self
            .events
            .iter()
            .take_while(|(event_at, _)| *event_at <= cutoff)
            .map(|(_, weight)| u64::from(*weight))
            .sum::<u64>();
        Ok(Some(self.retained_weight - expired))
    }

    /// Whether the configured bound can currently represent the full window.
    pub fn is_complete_at(&self, at: EventTimeMicros) -> Result<bool, WindowError> {
        Ok(self.count_at(at)?.is_some())
    }
}

fn decay(elapsed_micros: u64, tau_micros: u64) -> f64 {
    (-(elapsed_micros as f64) / tau_micros as f64).exp()
}

/// Stable identity for a message author in a chat snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FloodActor {
    User(i64),
    SenderChat(i64),
}

/// Latest projected message input for a bounded conversation window.
///
/// `tokens` must already be normalized by the caller. The core deliberately
/// receives no Telegram object, raw text, persistence handle, or policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationMessage {
    pub message_id: i64,
    pub event_at: EventTimeMicros,
    pub actor: FloodActor,
    pub reply_to: Option<i64>,
    /// Telegram's shared media-group identity; album items count as one send.
    pub album_id: Option<String>,
    pub tokens: Vec<String>,
    pub media_items: u32,
    pub emoji_count: u32,
    pub mention_count: u32,
    pub forwarded: bool,
}

/// Safety bounds applied before allocating window-analysis state.
pub const MAX_WINDOW_MESSAGES: usize = 2_048;
pub const MAX_TOKENS_PER_MESSAGE: usize = 512;
pub const MAX_TOTAL_TOKENS: usize = 16_384;
pub const MAX_TOKEN_BYTES: usize = 256;

/// Invalid or oversized input to [`analyze_window`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowInputError {
    ZeroDuration,
    TooManyMessages,
    TooManyTokens,
    TokenTooLong,
    DuplicateMessageId,
}

impl fmt::Display for WindowInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDuration => f.write_str("conversation window duration must be positive"),
            Self::TooManyMessages => f.write_str("conversation window message limit exceeded"),
            Self::TooManyTokens => f.write_str("conversation window token limit exceeded"),
            Self::TokenTooLong => f.write_str("conversation token length limit exceeded"),
            Self::DuplicateMessageId => {
                f.write_str("conversation snapshot contains a duplicate message id")
            }
        }
    }
}

impl std::error::Error for WindowInputError {}

/// Typed observation names. These values are evidence only, never actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloodMetricKind {
    LogicalMessages,
    MessagesPerSecond,
    MediaItems,
    EmojiItems,
    MentionItems,
    ForwardedMessages,
    LargestActorShare,
    ConsecutiveActorRun,
    ConversationTurns,
    TokenEntropyBits,
    ExactDuplicatePairs,
    MaximumTrigramJaccard,
    ContentNovelty,
    ReplyTargetDiversity,
    MessagesPerToken,
    MaximumCrossActorJaccard,
}

/// A single numeric feature and whether its input snapshot is complete.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloodMetric {
    pub kind: FloodMetricKind,
    pub value: Option<f64>,
    pub complete: bool,
}

/// Evidence-only result for a bounded latest-message snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct FloodAssessment {
    pub metrics: Vec<FloodMetric>,
    pub complete: bool,
}

impl FloodAssessment {
    fn push(&mut self, kind: FloodMetricKind, value: Option<f64>) {
        self.metrics.push(FloodMetric {
            kind,
            value,
            complete: self.complete,
        });
    }

    /// Finds one metric by its stable typed name.
    pub fn metric(&self, kind: FloodMetricKind) -> Option<FloodMetric> {
        self.metrics
            .iter()
            .copied()
            .find(|metric| metric.kind == kind)
    }
}

/// Summarizes a latest-projection snapshot for `(now - duration, now]`.
///
/// The caller is responsible for supplying one current revision per message,
/// scoping the snapshot to one instance/chat/topic, and setting `complete` to
/// false when database history or state is missing. No metric is itself a
/// verdict; notably burst, repetition, and actor share are only observations.
pub fn analyze_window(
    messages: &[ConversationMessage],
    now: EventTimeMicros,
    duration_micros: u64,
    complete: bool,
) -> Result<FloodAssessment, WindowInputError> {
    if duration_micros == 0 {
        return Err(WindowInputError::ZeroDuration);
    }
    if messages.len() > MAX_WINDOW_MESSAGES {
        return Err(WindowInputError::TooManyMessages);
    }

    let mut seen_message_ids = HashSet::with_capacity(messages.len());
    let mut total_tokens = 0usize;
    for message in messages {
        if !seen_message_ids.insert(message.message_id) {
            return Err(WindowInputError::DuplicateMessageId);
        }
        if message.tokens.len() > MAX_TOKENS_PER_MESSAGE {
            return Err(WindowInputError::TooManyTokens);
        }
        total_tokens = total_tokens.saturating_add(message.tokens.len());
        if total_tokens > MAX_TOTAL_TOKENS {
            return Err(WindowInputError::TooManyTokens);
        }
        if message
            .tokens
            .iter()
            .any(|token| token.len() > MAX_TOKEN_BYTES)
        {
            return Err(WindowInputError::TokenTooLong);
        }
    }

    let cutoff = now.checked_sub(duration_micros);
    let mut selected: Vec<_> = messages
        .iter()
        .filter(|message| {
            message.event_at <= now && cutoff.is_none_or(|lower| message.event_at > lower)
        })
        .collect();
    selected.sort_by_key(|message| (message.event_at, message.message_id));

    let mut assessment = FloodAssessment {
        metrics: Vec::with_capacity(16),
        complete,
    };
    let mut seen_logical = HashSet::with_capacity(selected.len());
    let logical_events: Vec<_> = selected
        .iter()
        .filter_map(|message| {
            let key = match message.album_id.as_deref() {
                Some(album) => LogicalMessageKey::Album(album),
                None => LogicalMessageKey::Message(message.message_id),
            };
            seen_logical
                .insert((message.actor, key.clone()))
                .then_some((message.actor, key))
        })
        .collect();
    let logical_messages = logical_events.len();
    let seconds = duration_micros as f64 / 1_000_000.0;
    assessment.push(
        FloodMetricKind::LogicalMessages,
        Some(logical_messages as f64),
    );
    assessment.push(
        FloodMetricKind::MessagesPerSecond,
        Some(logical_messages as f64 / seconds),
    );
    assessment.push(
        FloodMetricKind::MediaItems,
        Some(
            selected
                .iter()
                .map(|message| u64::from(message.media_items))
                .sum::<u64>() as f64,
        ),
    );
    assessment.push(
        FloodMetricKind::EmojiItems,
        Some(
            selected
                .iter()
                .map(|message| u64::from(message.emoji_count))
                .sum::<u64>() as f64,
        ),
    );
    assessment.push(
        FloodMetricKind::MentionItems,
        Some(
            selected
                .iter()
                .map(|message| u64::from(message.mention_count))
                .sum::<u64>() as f64,
        ),
    );
    assessment.push(
        FloodMetricKind::ForwardedMessages,
        Some(selected.iter().filter(|message| message.forwarded).count() as f64),
    );

    let mut actor_logical = HashMap::<FloodActor, usize>::new();
    let mut previous_actor = None;
    let mut consecutive = 0usize;
    let mut maximum_consecutive = 0usize;
    let mut turns = 0usize;
    for (actor, _) in &logical_events {
        *actor_logical.entry(*actor).or_default() += 1;
        if previous_actor == Some(*actor) {
            consecutive += 1;
        } else {
            if previous_actor.is_some() {
                turns += 1;
            }
            previous_actor = Some(*actor);
            consecutive = 1;
        }
        maximum_consecutive = maximum_consecutive.max(consecutive);
    }
    let largest_actor_logical = actor_logical.values().copied().max().unwrap_or(0);
    assessment.push(
        FloodMetricKind::LargestActorShare,
        (logical_messages > 0).then(|| largest_actor_logical as f64 / logical_messages as f64),
    );
    assessment.push(
        FloodMetricKind::ConsecutiveActorRun,
        (!selected.is_empty()).then_some(maximum_consecutive as f64),
    );
    assessment.push(
        FloodMetricKind::ConversationTurns,
        (!selected.is_empty()).then_some(turns as f64),
    );

    let mut token_counts = HashMap::<&str, usize>::new();
    let mut prior_tokens = HashSet::<&str>::new();
    let mut fresh_tokens = 0usize;
    let mut token_occurrences = 0usize;
    let mut unique_reply_targets = HashSet::<i64>::new();
    let mut reply_count = 0usize;
    let mut duplicates_by_context = HashMap::<(Option<i64>, &[String]), usize>::new();
    let mut unique_token_sets = Vec::<(FloodActor, HashSet<&str>)>::new();
    let mut message_ngrams = Vec::<HashSet<Vec<&str>>>::new();
    let mut seen_logical_context = HashSet::with_capacity(logical_messages);
    for message in &selected {
        let logical_key = match message.album_id.as_deref() {
            Some(album) => LogicalMessageKey::Album(album),
            None => LogicalMessageKey::Message(message.message_id),
        };
        if seen_logical_context.insert((message.actor, logical_key))
            && let Some(reply_to) = message.reply_to
        {
            reply_count += 1;
            unique_reply_targets.insert(reply_to);
        }
        if !message.tokens.is_empty() {
            *duplicates_by_context
                .entry((message.reply_to, message.tokens.as_slice()))
                .or_default() += 1;
            let mut token_set = HashSet::new();
            for token in &message.tokens {
                *token_counts.entry(token.as_str()).or_default() += 1;
                fresh_tokens += usize::from(!prior_tokens.contains(token.as_str()));
                token_set.insert(token.as_str());
                token_occurrences += 1;
            }
            prior_tokens.extend(token_set.iter().copied());
            unique_token_sets.push((message.actor, token_set));
        }
        let ngrams: HashSet<_> = message
            .tokens
            .windows(3)
            .map(|ngram| ngram.iter().map(String::as_str).collect())
            .collect();
        if !ngrams.is_empty() {
            message_ngrams.push(ngrams);
        }
    }
    let entropy = if token_occurrences > 1 && token_counts.len() > 1 {
        let total = token_occurrences as f64;
        Some(
            token_counts
                .values()
                .map(|count| {
                    let probability = *count as f64 / total;
                    -probability * probability.log2()
                })
                .sum(),
        )
    } else {
        None
    };
    assessment.push(FloodMetricKind::TokenEntropyBits, entropy);
    assessment.push(
        FloodMetricKind::ExactDuplicatePairs,
        Some(
            duplicates_by_context
                .values()
                .map(|count| count.saturating_mul(count.saturating_sub(1)) / 2)
                .sum::<usize>() as f64,
        ),
    );
    assessment.push(
        FloodMetricKind::MaximumTrigramJaccard,
        maximum_pairwise_jaccard(&message_ngrams),
    );
    assessment.push(
        FloodMetricKind::ContentNovelty,
        (token_occurrences > 0).then(|| fresh_tokens as f64 / token_occurrences as f64),
    );
    assessment.push(
        FloodMetricKind::ReplyTargetDiversity,
        (reply_count > 0).then(|| unique_reply_targets.len() as f64 / reply_count as f64),
    );
    assessment.push(
        FloodMetricKind::MessagesPerToken,
        (token_occurrences > 0).then(|| logical_messages as f64 / token_occurrences as f64),
    );
    assessment.push(
        FloodMetricKind::MaximumCrossActorJaccard,
        maximum_cross_actor_jaccard(&unique_token_sets),
    );

    Ok(assessment)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum LogicalMessageKey<'a> {
    Message(i64),
    Album(&'a str),
}

fn maximum_pairwise_jaccard(sets: &[HashSet<Vec<&str>>]) -> Option<f64> {
    if sets.len() < 2 {
        return None;
    }
    let mut maximum: f64 = 0.0;
    for (index, left) in sets.iter().enumerate() {
        for right in &sets[index + 1..] {
            let union = left.union(right).count();
            if union > 0 {
                maximum = maximum.max(left.intersection(right).count() as f64 / union as f64);
            }
        }
    }
    Some(maximum)
}

fn maximum_cross_actor_jaccard(sets: &[(FloodActor, HashSet<&str>)]) -> Option<f64> {
    if sets.len() < 2 {
        return None;
    }
    let mut maximum: f64 = 0.0;
    let mut compared = false;
    for (index, (left_actor, left)) in sets.iter().enumerate() {
        for (right_actor, right) in &sets[index + 1..] {
            if left_actor == right_actor {
                continue;
            }
            let union = left.union(right).count();
            if union > 0 {
                compared = true;
                maximum = maximum.max(left.intersection(right).count() as f64 / union as f64);
            }
        }
    }
    compared.then_some(maximum)
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationMessage, DecayedMean, EwmaError, EwmaRate, FloodActor, FloodMetricKind,
        WindowInputError, analyze_window,
    };

    fn message(
        message_id: i64,
        actor: FloodActor,
        tokens: &[&str],
        reply_to: Option<i64>,
    ) -> ConversationMessage {
        ConversationMessage {
            message_id,
            event_at: message_id as u64 * 1_000_000,
            actor,
            reply_to,
            album_id: None,
            tokens: tokens.iter().map(|token| (*token).to_owned()).collect(),
            media_items: 0,
            emoji_count: 0,
            mention_count: 0,
            forwarded: false,
        }
    }

    #[test]
    fn rate_uses_event_weight_and_decay_at_equal_and_later_timestamps() {
        let mut rate = EwmaRate::new(1_000_000).unwrap();
        assert_eq!(rate.rate_at(0).unwrap(), None);
        rate.observe(10, 1).unwrap();
        rate.observe(10, 2).unwrap();
        assert!((rate.rate_at(10).unwrap().unwrap() - 3.0).abs() < 1e-12);
        let after_one_tau = rate.rate_at(1_000_010).unwrap().unwrap();
        assert!((after_one_tau - 3.0 / std::f64::consts::E).abs() < 1e-12);
        assert_eq!(rate.accepted_events(), 3);
    }

    #[test]
    fn rate_halves_at_the_exponential_half_life() {
        let mut rate = EwmaRate::new(1_000_000).unwrap();
        rate.observe(0, 1).unwrap();
        let half_life_micros = (1_000_000.0 * std::f64::consts::LN_2).round() as u64;
        let observed = rate.rate_at(half_life_micros).unwrap().unwrap();
        assert!((observed - 0.5).abs() < 1e-6);
    }

    #[test]
    fn invalid_rate_updates_do_not_mutate_state() {
        assert_eq!(EwmaRate::new(0), Err(EwmaError::ZeroTimeConstant));
        let mut rate = EwmaRate::new(5).unwrap();
        rate.observe(10, 2).unwrap();
        let before = rate.clone();
        assert_eq!(rate.observe(11, 0), Err(EwmaError::ZeroEventWeight));
        assert_eq!(
            rate.observe(9, 1),
            Err(EwmaError::OutOfOrder {
                last_accepted: 10,
                received: 9,
            })
        );
        assert_eq!(rate, before);
    }

    #[test]
    fn rate_read_does_not_advance_internal_time() {
        let mut observed_after_read = EwmaRate::new(2_000_000).unwrap();
        let mut direct = EwmaRate::new(2_000_000).unwrap();
        observed_after_read.observe(0, 1).unwrap();
        direct.observe(0, 1).unwrap();
        let _ = observed_after_read.rate_at(1_000_000).unwrap();
        observed_after_read.observe(2_000_000, 1).unwrap();
        direct.observe(2_000_000, 1).unwrap();
        assert_eq!(observed_after_read, direct);
    }

    #[test]
    fn rate_matches_independent_full_history_sum() {
        let tau = 7_300_000_u64;
        let mut rate = EwmaRate::new(tau).unwrap();
        let mut observations = Vec::new();
        let mut seed = 0x9e37_79b9_u64;
        let mut at = 0_u64;
        for _ in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            at += seed % 31_000 + 1;
            let weight = ((seed >> 24) % 4 + 1) as u32;
            observations.push((at, weight));
            rate.observe(at, weight).unwrap();
        }

        for extra in [0, 1, tau / 2, tau, tau * 7] {
            let now = at + extra;
            let expected = observations
                .iter()
                .map(|(time, weight)| {
                    f64::from(*weight) * 1_000_000.0 / tau as f64
                        * (-((now - time) as f64) / tau as f64).exp()
                })
                .sum::<f64>();
            let actual = rate.rate_at(now).unwrap().unwrap();
            assert!((actual - expected).abs() <= 1e-10 * expected.max(1.0));
        }
    }

    #[test]
    fn weighted_mean_returns_unknown_until_warm_and_preserves_ratio_under_decay() {
        let mut mean = DecayedMean::new(1_000_000, 1.5).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), None);
        mean.observe(0, 10.0, 1.0).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), None);
        mean.observe(0, 20.0, 1.0).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), Some(15.0));
        assert_eq!(mean.mean_at(100_000).unwrap(), Some(15.0));
        assert_eq!(mean.mean_at(2_000_000).unwrap(), None);
        assert_eq!(mean.accepted_samples(), 2);
    }

    #[test]
    fn weighted_mean_rejects_nonfinite_and_out_of_order_input_atomically() {
        assert_eq!(DecayedMean::new(0, 1.0), Err(EwmaError::ZeroTimeConstant));
        assert_eq!(DecayedMean::new(1, 0.0), Err(EwmaError::InvalidSample));
        let mut mean = DecayedMean::new(1_000, 0.1).unwrap();
        mean.observe(100, 3.0, 1.0).unwrap();
        let before = mean.clone();
        assert_eq!(
            mean.observe(100, f64::NAN, 1.0),
            Err(EwmaError::InvalidSample)
        );
        assert_eq!(
            mean.observe(100, 3.0, f64::INFINITY),
            Err(EwmaError::InvalidSample)
        );
        assert_eq!(
            mean.observe(99, 4.0, 1.0),
            Err(EwmaError::OutOfOrder {
                last_accepted: 100,
                received: 99,
            })
        );
        assert_eq!(mean, before);
    }

    #[test]
    fn weighted_mean_matches_full_history_weighted_sum() {
        let tau = 11_000_u64;
        let min_weight = 0.01;
        let mut mean = DecayedMean::new(tau, min_weight).unwrap();
        let mut observations = Vec::new();
        let mut seed = 0x1234_5678_u64;
        let mut at = 0_u64;
        for _ in 0..300 {
            seed = seed
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            at += seed % 900 + 1;
            let value = ((seed >> 18) % 20_000) as f64 / 100.0 - 50.0;
            let weight = ((seed >> 43) % 100 + 1) as f64 / 10.0;
            observations.push((at, value, weight));
            mean.observe(at, value, weight).unwrap();
        }

        let now = at + tau / 3;
        let (sum, weight) = observations
            .iter()
            .map(|(time, value, sample_weight)| {
                let decay = (-((now - time) as f64) / tau as f64).exp();
                (value * sample_weight * decay, sample_weight * decay)
            })
            .fold((0.0, 0.0), |(sum, weight), (s, w)| (sum + s, weight + w));
        assert!((mean.mean_at(now).unwrap().unwrap() - sum / weight).abs() < 1e-10);
    }

    #[test]
    fn exact_window_uses_open_lower_and_closed_upper_boundaries() {
        let mut window = super::ExactWindow::new(1_000, 4).unwrap();
        assert_eq!(window.observe(0, 1), Ok(Some(1)));
        assert_eq!(window.observe(999, 2), Ok(Some(3)));
        assert_eq!(window.count_at(999), Ok(Some(3)));
        assert_eq!(window.observe(1_000, 1), Ok(Some(3)));
        assert_eq!(window.count_at(1_000), Ok(Some(3)));
    }

    #[test]
    fn exact_window_reports_incomplete_until_overflowed_event_expires() {
        use super::WindowError;

        let mut window = super::ExactWindow::new(10, 1).unwrap();
        assert_eq!(window.observe(2, 1), Ok(Some(1)));
        assert_eq!(window.observe(3, 1), Err(WindowError::CapacityExceeded));
        assert_eq!(window.count_at(12), Ok(None));
        assert_eq!(window.count_at(13), Ok(Some(0)));
        assert_eq!(window.observe(13, 1), Ok(Some(1)));
    }

    #[test]
    fn exact_window_observe_does_not_report_a_partial_count_after_capacity_loss() {
        use super::WindowError;

        let mut window = super::ExactWindow::new(10, 1).unwrap();
        assert_eq!(window.observe(0, 1), Ok(Some(1)));
        assert_eq!(window.observe(1, 1), Err(WindowError::CapacityExceeded));
        assert_eq!(window.observe(10, 1), Ok(None));
        assert_eq!(window.count_at(10), Ok(None));
        assert_eq!(window.observe(20, 1), Ok(Some(1)));
    }

    #[test]
    fn exact_window_invalid_input_does_not_change_state() {
        use super::WindowError;

        assert_eq!(
            super::ExactWindow::new(0, 1).unwrap_err(),
            WindowError::ZeroWindow
        );
        assert_eq!(
            super::ExactWindow::new(1, 0).unwrap_err(),
            WindowError::ZeroCapacity
        );
        let mut window = super::ExactWindow::new(100, 2).unwrap();
        window.observe(10, 2).unwrap();
        let before = window.clone();
        assert_eq!(window.observe(11, 0), Err(WindowError::ZeroEventWeight));
        assert_eq!(
            window.observe(9, 1),
            Err(WindowError::OutOfOrder {
                last_accepted: 10,
                received: 9,
            })
        );
        assert_eq!(window, before);
    }

    #[test]
    fn exact_window_count_overflow_does_not_prune_or_advance_time() {
        let mut window = super::ExactWindow::new(100, 2).unwrap();
        window.observe(10, 2).unwrap();
        window.retained_weight = u64::MAX;
        let before = window.clone();

        assert_eq!(
            window.observe(109, 3),
            Err(super::WindowError::CountOverflow)
        );
        assert_eq!(window, before);
    }

    #[test]
    fn exact_window_matches_full_history_reference() {
        let duration = 17_000_u64;
        let mut window = super::ExactWindow::new(duration, 1_000).unwrap();
        let mut events = Vec::new();
        let mut at = 0_u64;
        let mut seed = 0xfeed_beef_u64;
        for _ in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            at += seed % 2_000 + 1;
            let weight = ((seed >> 20) % 4 + 1) as u32;
            events.push((at, weight));
            window.observe(at, weight).unwrap();
        }
        for extra in [0, 1, duration / 2, duration, duration * 3] {
            let now = at + extra;
            let expected = events
                .iter()
                .filter(|(time, _)| *time > now.saturating_sub(duration) && *time <= now)
                .map(|(_, weight)| u64::from(*weight))
                .sum::<u64>();
            assert_eq!(window.count_at(now), Ok(Some(expected)));
        }
    }

    #[test]
    fn media_group_counts_as_one_logical_send_and_keeps_all_items() {
        let mut first = message(1, FloodActor::User(10), &[], None);
        first.album_id = Some("album-1".to_owned());
        first.media_items = 1;
        let mut second = message(2, FloodActor::User(10), &[], None);
        second.album_id = Some("album-1".to_owned());
        second.media_items = 2;
        let third = message(3, FloodActor::User(20), &[], None);

        let result = analyze_window(&[first, second, third], 10_000_000, 10_000_000, true).unwrap();
        assert_eq!(
            result
                .metric(FloodMetricKind::LogicalMessages)
                .unwrap()
                .value,
            Some(2.0)
        );
        assert_eq!(
            result.metric(FloodMetricKind::MediaItems).unwrap().value,
            Some(3.0)
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::LargestActorShare)
                .unwrap()
                .value,
            Some(0.5)
        );
    }

    #[test]
    fn repeated_short_replies_to_different_targets_are_not_exact_duplicates() {
        let messages = [
            message(1, FloodActor::User(10), &["fedora"], Some(100)),
            message(2, FloodActor::User(10), &["fedora"], Some(200)),
        ];
        let result = analyze_window(&messages, 10_000_000, 10_000_000, true).unwrap();
        assert_eq!(
            result
                .metric(FloodMetricKind::ExactDuplicatePairs)
                .unwrap()
                .value,
            Some(0.0)
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::MaximumTrigramJaccard)
                .unwrap()
                .value,
            None
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::ReplyTargetDiversity)
                .unwrap()
                .value,
            Some(1.0)
        );
    }

    #[test]
    fn exact_duplicates_require_the_same_reply_context_and_are_only_evidence() {
        let messages = [
            message(1, FloodActor::User(10), &["buy", "cheap", "now"], Some(100)),
            message(2, FloodActor::User(10), &["buy", "cheap", "now"], Some(100)),
        ];
        let result = analyze_window(&messages, 10_000_000, 10_000_000, false).unwrap();
        assert!(!result.complete);
        assert!(result.metrics.iter().all(|metric| !metric.complete));
        assert_eq!(
            result
                .metric(FloodMetricKind::ExactDuplicatePairs)
                .unwrap()
                .value,
            Some(1.0)
        );
    }

    #[test]
    fn informative_monologue_has_features_but_no_verdict_or_policy() {
        let messages = [
            message(1, FloodActor::User(10), &["first", "new", "fact"], None),
            message(2, FloodActor::User(10), &["second", "new", "fact"], None),
            message(3, FloodActor::User(10), &["third", "useful", "fact"], None),
        ];
        let result = analyze_window(&messages, 10_000_000, 10_000_000, true).unwrap();
        assert_eq!(
            result
                .metric(FloodMetricKind::LargestActorShare)
                .unwrap()
                .value,
            Some(1.0)
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::ExactDuplicatePairs)
                .unwrap()
                .value,
            Some(0.0)
        );
        assert!(
            result
                .metric(FloodMetricKind::TokenEntropyBits)
                .unwrap()
                .value
                .is_some()
        );
    }

    #[test]
    fn empty_snapshot_reports_unknown_context_features_and_zero_counts() {
        let result = analyze_window(&[], 10_000_000, 5_000_000, true).unwrap();
        assert_eq!(
            result
                .metric(FloodMetricKind::LogicalMessages)
                .unwrap()
                .value,
            Some(0.0)
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::MessagesPerSecond)
                .unwrap()
                .value,
            Some(0.0)
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::TokenEntropyBits)
                .unwrap()
                .value,
            None
        );
        assert_eq!(
            result
                .metric(FloodMetricKind::ContentNovelty)
                .unwrap()
                .value,
            None
        );
    }

    #[test]
    fn window_input_rejects_duplicate_ids_and_oversized_token_data() {
        let duplicate = [
            message(7, FloodActor::User(10), &["a"], None),
            message(7, FloodActor::User(20), &["b"], None),
        ];
        assert_eq!(
            analyze_window(&duplicate, 10_000_000, 10_000_000, true).unwrap_err(),
            WindowInputError::DuplicateMessageId
        );
        let mut too_long = message(8, FloodActor::User(10), &["a"], None);
        too_long.tokens[0] = "x".repeat(super::MAX_TOKEN_BYTES + 1);
        assert_eq!(
            analyze_window(&[too_long], 10_000_000, 10_000_000, true).unwrap_err(),
            WindowInputError::TokenTooLong
        );
    }
}
