//! Backpressure-aware Responses SSE normalization. EOF without a terminal
//! response is an error; a terminal event is emitted only after final parts.

use std::collections::{BTreeMap, VecDeque};
use std::io;

use eventsource_stream::Eventsource;
use futures_util::stream::{self, BoxStream, StreamExt};
use loom_core::ContentPart;
use loom_provider::{ContentDelta, ProviderError, TurnEvent, TurnEventKind, TurnEventStream};
use serde_json::{json, Value};

use crate::{translate, PROVIDER_NAME};

type Key = (usize, Option<usize>);
struct Part {
    index: usize,
    value: ContentPart,
    done: bool,
}

#[derive(Default)]
struct Accumulator {
    parts: BTreeMap<Key, Part>,
    queue: VecDeque<Result<TurnEvent, ProviderError>>,
    started: bool,
    ended: bool,
    next_output: usize,
    pending: BTreeMap<usize, VecDeque<Value>>,
}

impl Accumulator {
    fn emit(&mut self, kind: TurnEventKind, raw: &Value) {
        self.queue.push_back(Ok(TurnEvent::new(kind, raw.clone())));
    }

    fn start(&mut self, key: Key, value: ContentPart, raw: &Value) -> Result<(), ProviderError> {
        if self.parts.contains_key(&key) {
            return Ok(());
        }
        if self.parts.len() >= 16384 {
            return Err(translate::invalid("too many OpenAI content parts"));
        }
        let index = self.parts.len();
        self.parts.insert(
            key,
            Part {
                index,
                value: value.clone(),
                done: false,
            },
        );
        self.emit(
            TurnEventKind::ContentPartStarted { index, part: value },
            raw,
        );
        Ok(())
    }

    fn complete(&mut self, key: Key, value: ContentPart, raw: &Value) -> Result<(), ProviderError> {
        self.start(key, value.clone(), raw)?;
        let part = self.parts.get_mut(&key).expect("start creates a part");
        if part.done {
            if part.value != value {
                return Err(translate::invalid("inconsistent OpenAI completed part"));
            }
            return Ok(());
        }
        part.done = true;
        part.value = value.clone();
        let index = part.index;
        self.emit(
            TurnEventKind::ContentPartComplete { index, part: value },
            raw,
        );
        Ok(())
    }

    fn delta(&mut self, key: Key, delta: ContentDelta, raw: &Value) -> Result<(), ProviderError> {
        let part = self
            .parts
            .get(&key)
            .ok_or_else(|| translate::invalid("OpenAI delta preceded its content part"))?;
        if part.done {
            return Err(translate::invalid(
                "OpenAI delta followed its completed part",
            ));
        }
        let index = part.index;
        self.emit(TurnEventKind::ContentPartDelta { index, delta }, raw);
        Ok(())
    }

    fn finish_item(
        &mut self,
        index: usize,
        item: &Value,
        raw: &Value,
    ) -> Result<(), ProviderError> {
        let parts = translate::output_item(item)?;
        for (position, part) in parts.into_iter().enumerate() {
            let key = (index, position.checked_sub(1));
            self.complete(key, part, raw)?;
        }
        Ok(())
    }

    fn ingest(&mut self, raw: Value) -> Result<(), ProviderError> {
        // A later parallel output may arrive before the previous message's
        // content count is known. Preserve its raw event immediately, but defer
        // assigning normalized flat indices until preceding output completes.
        let mut ready = VecDeque::from([raw]);
        while let Some(raw) = ready.pop_front() {
            let output = raw
                .get("output_index")
                .map(|_| index(&raw, "output_index"))
                .transpose()?;
            if let Some(output) = output {
                if output > 16384 {
                    return Err(translate::invalid("OpenAI output index exceeds limit"));
                }
                if output > self.next_output {
                    let native_type = raw.get("type").and_then(Value::as_str).map(str::to_owned);
                    self.emit(TurnEventKind::Other { native_type }, &raw);
                    self.pending.entry(output).or_default().push_back(raw);
                    continue;
                }
            }
            let completed =
                raw.get("type").and_then(Value::as_str) == Some("response.output_item.done");
            self.process(raw)?;
            if completed && output == Some(self.next_output) {
                self.next_output += 1;
                if let Some(mut pending) = self.pending.remove(&self.next_output) {
                    pending.append(&mut ready);
                    ready = pending;
                }
            }
        }
        Ok(())
    }

    fn process(&mut self, raw: Value) -> Result<(), ProviderError> {
        let native_type = raw
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| translate::invalid("OpenAI event has no type"))?;
        if self.ended {
            return Err(translate::invalid(
                "OpenAI event followed the terminal response",
            ));
        }
        if native_type == "error" || native_type == "response.failed" {
            return Err(translate::response_error(raw));
        }
        if native_type == "response.created" {
            if self.started {
                return Err(translate::invalid("OpenAI response started twice"));
            }
            self.started = true;
            self.emit(TurnEventKind::TurnStarted, &raw);
            return Ok(());
        }
        if !self.started {
            return Err(translate::invalid("OpenAI stream omitted response.created"));
        }
        match native_type {
            "response.output_item.added" => {
                let output = index(&raw, "output_index")?;
                let item = raw
                    .get("item")
                    .ok_or_else(|| translate::invalid("missing OpenAI output item"))?;
                match item.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        self.start((output, None), translate::metadata(item), &raw)?
                    }
                    Some("function_call") => {
                        let header = ContentPart::ProviderExtension {
                            provider: PROVIDER_NAME.into(),
                            kind: "function_call_metadata".into(),
                            payload: item.clone(),
                        };
                        self.start((output, None), header, &raw)?;
                        let part = ContentPart::ToolUse {
                            id: field(item, "call_id")?,
                            name: field(item, "name")?,
                            input: json!({}),
                            cache: None,
                        };
                        self.start((output, Some(0)), part, &raw)?;
                    }
                    Some(kind) => {
                        let part = ContentPart::ProviderExtension {
                            provider: PROVIDER_NAME.into(),
                            kind: kind.into(),
                            payload: item.clone(),
                        };
                        self.start((output, None), part, &raw)?;
                    }
                    None => return Err(translate::invalid("missing OpenAI output item type")),
                }
            }
            "response.content_part.added" => {
                let part = translate::content_part(
                    raw.get("part")
                        .ok_or_else(|| translate::invalid("missing OpenAI content part"))?,
                )?;
                self.start(
                    (
                        index(&raw, "output_index")?,
                        Some(index(&raw, "content_index")?),
                    ),
                    part,
                    &raw,
                )?;
            }
            "response.output_text.delta" => {
                self.delta(
                    (
                        index(&raw, "output_index")?,
                        Some(index(&raw, "content_index")?),
                    ),
                    ContentDelta::Text {
                        text: field(&raw, "delta")?,
                    },
                    &raw,
                )?;
            }
            "response.function_call_arguments.delta" => {
                self.delta(
                    (index(&raw, "output_index")?, Some(0)),
                    ContentDelta::Json {
                        partial_json: field(&raw, "delta")?,
                    },
                    &raw,
                )?;
            }
            "response.content_part.done" => {
                let part = translate::content_part(
                    raw.get("part")
                        .ok_or_else(|| translate::invalid("missing completed OpenAI content"))?,
                )?;
                self.complete(
                    (
                        index(&raw, "output_index")?,
                        Some(index(&raw, "content_index")?),
                    ),
                    part,
                    &raw,
                )?;
            }
            "response.output_item.done" => {
                self.finish_item(
                    index(&raw, "output_index")?,
                    raw.get("item")
                        .ok_or_else(|| translate::invalid("missing completed OpenAI item"))?,
                    &raw,
                )?;
            }
            "response.completed" | "response.incomplete" => {
                let response = raw
                    .get("response")
                    .ok_or_else(|| translate::invalid("missing terminal OpenAI response"))?;
                let expected = if native_type == "response.completed" {
                    "completed"
                } else {
                    "incomplete"
                };
                if response.get("status").and_then(Value::as_str) != Some(expected) {
                    return Err(translate::invalid("inconsistent OpenAI terminal status"));
                }
                let items = response
                    .get("output")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        translate::invalid("terminal OpenAI response has no output array")
                    })?;
                let mut count = 0;
                for (position, item) in items.iter().enumerate() {
                    count += translate::output_item(item)?.len();
                    self.finish_item(position, item, &raw)?;
                }
                if self.parts.len() != count || self.parts.values().any(|part| !part.done) {
                    return Err(translate::invalid(
                        "OpenAI stream does not match its final response",
                    ));
                }
                let usage = response
                    .get("usage")
                    .filter(|v| !v.is_null())
                    .map(translate::usage);
                self.emit(
                    TurnEventKind::TurnEnded {
                        stop_reason: translate::stop_reason(response),
                        usage,
                        cost: None,
                    },
                    &raw,
                );
                self.ended = true;
            }
            _ => self.emit(
                TurnEventKind::Other {
                    native_type: Some(native_type.into()),
                },
                &raw,
            ),
        }
        Ok(())
    }
}

fn index(value: &Value, name: &str) -> Result<usize, ProviderError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| translate::invalid("invalid OpenAI event index"))
}
fn field(value: &Value, name: &str) -> Result<String, ProviderError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| translate::invalid("missing OpenAI event string"))
}

struct State {
    events: BoxStream<
        'static,
        Result<eventsource_stream::Event, eventsource_stream::EventStreamError<io::Error>>,
    >,
    accumulator: Accumulator,
    done: bool,
}

pub(crate) fn event_stream(response: reqwest::Response) -> TurnEventStream {
    // Bound the native body before the SSE parser accumulates it. A malicious
    // unterminated frame cannot allocate indefinitely while waiting for EOF.
    let bytes = response.bytes_stream().scan(0usize, |total, chunk| {
        let result = match chunk {
            Ok(bytes) => {
                *total += bytes.len();
                if *total > 64 * 1024 * 1024 {
                    Err(io::Error::other("OpenAI stream exceeds size limit"))
                } else {
                    Ok(bytes)
                }
            }
            Err(_) => Err(io::Error::other("OpenAI stream transport interrupted")),
        };
        std::future::ready(Some(result))
    });
    let state = State {
        events: bytes.eventsource().boxed(),
        accumulator: Accumulator::default(),
        done: false,
    };
    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.accumulator.queue.pop_front() {
                return Some((event, state));
            }
            if state.done || state.accumulator.ended {
                return None;
            }
            match state.events.next().await {
                Some(Ok(event)) if event.data.is_empty() => continue,
                Some(Ok(event)) => {
                    if event.data.len() > 8 * 1024 * 1024 {
                        state.done = true;
                        return Some((
                            Err(translate::invalid("OpenAI event exceeds size limit")),
                            state,
                        ));
                    }
                    let parsed = serde_json::from_str(&event.data)
                        .map_err(|_| translate::invalid("invalid OpenAI SSE JSON"))
                        .and_then(|value| state.accumulator.ingest(value));
                    if let Err(error) = parsed {
                        state.accumulator.queue.clear();
                        state.done = true;
                        return Some((Err(error), state));
                    }
                }
                Some(Err(_)) => {
                    state.done = true;
                    return Some((
                        Err(ProviderError::Transport("OpenAI stream interrupted".into())),
                        state,
                    ));
                }
                None => {
                    state.done = true;
                    return Some((
                        Err(ProviderError::Transport(
                            "OpenAI stream ended before a terminal response".into(),
                        )),
                        state,
                    ));
                }
            }
        }
    })
    .boxed()
}
