//! Application state.

use crate::io::{self, IoRequest, IoResponse};
use base32::Alphabet;
use base32::encode as base32_encode;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ekman_core::models::{
    ActivityDay, ActivityRequest, GraphResponse, PopulatedExercise, PopulatedTemplate,
    SetForDayRequest,
};
use rand::{RngCore, rngs::OsRng};
use reqwest::cookie::Jar;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use urlencoding::encode as url_encode;

const INPUT_TIMEOUT: Duration = Duration::from_secs(1);
const ACTIVITY_WINDOW_DAYS: i64 = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Auth,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Login,
    Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Username,
    Password,
    Totp,
}

pub struct AuthState {
    pub mode: AuthMode,
    pub username: String,
    pub password: String,
    pub totp_code: String,
    pub status: String,
    pub focus: AuthField,
    pub submitting: bool,
    pub totp_secret: String,
    pub otpauth_url: String,
}

/// Main application state.
pub struct App {
    pub running: bool,
    pub view: View,
    pub auth: AuthState,
    cookie_store: Arc<Jar>,
    cookie_path: PathBuf,
    pub exercises: Vec<ExerciseState>,
    pub graphs: Vec<GraphResponse>,
    pub activity: Vec<ActivityDay>,
    pub selected: usize,
    pub status: StatusLine,
    io_tx: mpsc::Sender<IoRequest>,
    io_rx: mpsc::Receiver<IoResponse>,
    pending_graphs: HashSet<i64>,
}

impl App {
    pub fn new(
        io_tx: mpsc::Sender<IoRequest>,
        io_rx: mpsc::Receiver<IoResponse>,
        cookie_store: Arc<Jar>,
        cookie_path: PathBuf,
    ) -> Self {
        Self {
            running: true,
            view: View::Auth,
            auth: AuthState::new_register(),
            cookie_store,
            cookie_path,
            exercises: ExerciseState::defaults(),
            graphs: Vec::new(),
            activity: Vec::new(),
            selected: 0,
            status: StatusLine::default(),
            io_tx,
            io_rx,
            pending_graphs: HashSet::new(),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.view == View::Dashboard
    }

    pub fn try_resume_session(&mut self) {
        let _ = self.io_tx.try_send(IoRequest::CheckSession);
    }

    pub fn handle_auth_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.running = false,
            KeyCode::Tab => self.auth.next_field(),
            KeyCode::BackTab => self.auth.prev_field(),
            KeyCode::Enter => self.submit_auth(),
            KeyCode::Backspace => self.auth.backspace(),
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.switch_auth_mode(AuthMode::Login)
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.switch_auth_mode(AuthMode::Register);
                self.auth.regenerate_secret();
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.auth.mode == AuthMode::Register {
                    self.auth.regenerate_secret();
                }
            }
            KeyCode::Char(ch) => self.auth.push_char(ch),
            _ => {}
        }
    }

    fn submit_auth(&mut self) {
        if self.auth.submitting {
            return;
        }

        if self.auth.username.trim().is_empty() {
            self.auth.status = "Username is required".into();
            return;
        }
        if self.auth.password.trim().is_empty() {
            self.auth.status = "Password is required".into();
            return;
        }
        if self.auth.totp_code.trim().is_empty() {
            self.auth.status = "TOTP code is required".into();
            return;
        }

        self.auth.submitting = true;
        self.auth.status = match self.auth.mode {
            AuthMode::Login => "Signing in...".into(),
            AuthMode::Register => "Registering...".into(),
        };

        let _ = match self.auth.mode {
            AuthMode::Login => self.io_tx.try_send(IoRequest::Login {
                username: self.auth.username.clone(),
                password: self.auth.password.clone(),
                totp: self.auth.totp_code.clone(),
            }),
            AuthMode::Register => self.io_tx.try_send(IoRequest::Register {
                username: self.auth.username.clone(),
                password: self.auth.password.clone(),
                totp_secret: self.auth.totp_secret.clone(),
                totp_code: self.auth.totp_code.clone(),
            }),
        };
    }

    fn switch_auth_mode(&mut self, mode: AuthMode) {
        if self.auth.mode == mode {
            return;
        }

        self.auth.mode = mode;
        self.auth.totp_code.clear();
        self.auth.status.clear();
        self.auth.focus = AuthField::Username;
        if mode == AuthMode::Register {
            self.auth.regenerate_secret();
        }
    }

    fn on_authenticated(&mut self, username: String) {
        self.view = View::Dashboard;
        self.auth.submitting = false;
        self.auth.status.clear();
        self.status.backend = format!("Signed in as {username}");
        self.request_daily_plans();
        self.request_activity_history();
        self.refresh_status();
    }

    fn persist_cookies(&self) {
        if let Err(err) = io::save_session_cookie(&self.cookie_path, &self.cookie_store) {
            eprintln!("warning: failed to persist session cookies: {err}");
        }
    }

    pub fn request_daily_plans(&mut self) {
        if self.view != View::Dashboard {
            return;
        }
        self.status.backend = "Loading plans...".into();
        let _ = self.io_tx.try_send(IoRequest::LoadDailyPlans);
    }

    pub fn request_activity_history(&mut self) {
        if self.view != View::Dashboard {
            return;
        }
        let end_date = Utc::now().date_naive();
        let start_date = end_date - ChronoDuration::days(ACTIVITY_WINDOW_DAYS.saturating_sub(1));

        if let (Some(start), Some(end)) = (
            start_date
                .and_hms_opt(0, 0, 0)
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)),
            end_date
                .and_hms_opt(23, 59, 59)
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)),
        ) {
            let request = ActivityRequest {
                start: Some(start),
                end: Some(end),
            };
            let _ = self.io_tx.try_send(IoRequest::LoadActivityRange(request));
        }
    }

    pub fn poll_io(&mut self) {
        while let Ok(response) = self.io_rx.try_recv() {
            self.handle_response(response);
        }
    }

    fn handle_response(&mut self, response: IoResponse) {
        match response {
            IoResponse::LoggedIn(result) | IoResponse::Registered(result) => {
                self.auth.submitting = false;
                match result {
                    Ok(info) => {
                        self.on_authenticated(info.username);
                        self.persist_cookies();
                    }
                    Err(e) => {
                        self.auth.status = e;
                        self.view = View::Auth;
                    }
                }
            }
            IoResponse::SessionChecked(result) => match result {
                Ok(me) => {
                    self.on_authenticated(me.username);
                    self.persist_cookies();
                }
                Err(e) => {
                    self.auth.status = e;
                    self.view = View::Auth;
                }
            },
            IoResponse::DailyPlans(Ok(plans)) => self.apply_plans(plans),
            IoResponse::DailyPlans(Err(e)) => {
                self.status.backend = format!("Backend error: {e}");
                self.exercises = ExerciseState::defaults();
                self.graphs.clear();
            }
            IoResponse::Graph(id, Ok(graph)) => {
                self.pending_graphs.remove(&id);
                if let Some(existing) = self.graphs.iter_mut().find(|g| g.exercise_id == id) {
                    *existing = graph;
                } else {
                    self.graphs.push(graph);
                }
            }
            IoResponse::Graph(id, Err(e)) => {
                self.pending_graphs.remove(&id);
                self.status.backend = format!("Graph error for {id}: {e}");
            }
            IoResponse::Activity(Ok(activity)) => {
                self.activity = activity.days;
            }
            IoResponse::Activity(Err(e)) => {
                self.activity.clear();
                self.status.backend = format!("Activity error: {e}");
            }
            IoResponse::SetSaved {
                exercise_id,
                set_index,
                result,
            } => match result {
                Ok(saved) => {
                    if let Some(ex) = self
                        .exercises
                        .iter_mut()
                        .find(|e| e.exercise_id == Some(exercise_id))
                        && let Some(set) = ex.sets.get_mut(set_index)
                    {
                        set.remote_id = Some(saved.set_id);
                    }
                }
                Err(e) => {
                    self.status.backend = format!("Save set error: {e}");
                }
            },
        }
        if self.view == View::Dashboard {
            self.refresh_status();
        }
    }

    fn apply_plans(&mut self, plans: Vec<PopulatedTemplate>) {
        let plan = select_plan_for_today(&plans);

        if let Some(plan) = plan {
            let exercises: Vec<_> = plan
                .exercises
                .iter()
                .map(ExerciseState::from_populated)
                .collect();

            if exercises.is_empty() {
                self.status.backend = "No exercises in plan".into();
                self.exercises = ExerciseState::defaults();
            } else {
                self.status.backend = format!("Loaded: {}", plan.name);
                self.exercises = exercises;
                self.request_graphs();
            }
        } else {
            self.status.backend = "No plans available".into();
            self.exercises = ExerciseState::defaults();
        }

        self.selected = 0;
        self.graphs.clear();
        self.pending_graphs.clear();
    }

    fn request_graphs(&mut self) {
        let ids: Vec<i64> = self
            .exercises
            .iter()
            .filter_map(|ex| ex.exercise_id)
            .collect();

        for id in ids {
            self.request_graph(id);
        }
    }

    pub fn request_graph(&mut self, id: i64) {
        if self.pending_graphs.contains(&id) || self.graphs.iter().any(|g| g.exercise_id == id) {
            return;
        }
        if self.io_tx.try_send(IoRequest::LoadGraph(id)).is_ok() {
            self.pending_graphs.insert(id);
        }
    }

    pub fn current_exercise_mut(&mut self) -> Option<&mut ExerciseState> {
        self.exercises.get_mut(self.selected)
    }

    pub fn select_exercise(&mut self, delta: i32) {
        if self.exercises.is_empty() {
            return;
        }
        let len = self.exercises.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1);
        self.selected = next as usize;
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            ex.reset_input_timer();
        }
        self.refresh_status();
    }

    pub fn tab_next(&mut self) {
        if self.exercises.is_empty() {
            return;
        }

        if let Some(ex) = self.exercises.get_mut(self.selected) {
            match ex.focus {
                Focus::Weight => {
                    ex.focus = Focus::Reps;
                    return;
                }
                Focus::Reps if ex.set_cursor + 1 < ex.sets.len() => {
                    ex.set_cursor += 1;
                    ex.reset_input_timer();
                    return;
                }
                Focus::Reps => {}
            }
        }

        if self.selected + 1 < self.exercises.len() {
            self.selected += 1;
            if let Some(next) = self.exercises.get_mut(self.selected) {
                next.focus = Focus::Weight;
                next.set_cursor = 0;
                next.reset_input_timer();
            }
        }
        self.refresh_status();
    }

    pub fn tab_prev(&mut self) {
        if self.exercises.is_empty() {
            return;
        }

        if let Some(ex) = self.exercises.get_mut(self.selected) {
            match ex.focus {
                Focus::Reps => {
                    ex.focus = Focus::Weight;
                    return;
                }
                Focus::Weight if ex.set_cursor > 0 => {
                    ex.set_cursor -= 1;
                    ex.focus = Focus::Reps;
                    ex.reset_input_timer();
                    return;
                }
                Focus::Weight => {}
            }
        }

        if self.selected > 0 {
            self.selected -= 1;
            if let Some(prev) = self.exercises.get_mut(self.selected) {
                prev.set_cursor = prev.sets.len().saturating_sub(1);
                prev.focus = Focus::Reps;
                prev.reset_input_timer();
            }
        }
        self.refresh_status();
    }

    pub fn input_digit(&mut self, ch: char) {
        let Some(focus) = self.exercises.get(self.selected).map(|ex| ex.focus) else {
            return;
        };

        match focus {
            Focus::Weight => {
                if let Some(set_idx) = self.exercises.get_mut(self.selected).map(|ex| {
                    let set_idx = ex.set_cursor;
                    ex.push_weight_char(ch);
                    set_idx
                }) {
                    self.sync_set(self.selected, set_idx);
                }
            }
            Focus::Reps => {
                let target_exercise = self.selected;
                if self
                    .exercises
                    .get(target_exercise)
                    .is_some_and(|ex| ex.should_auto_advance())
                {
                    self.advance_set();
                }

                let target_set = self
                    .exercises
                    .get(target_exercise)
                    .map(|e| e.set_cursor)
                    .unwrap_or(0);
                let should_advance = self
                    .exercises
                    .get_mut(target_exercise)
                    .map(|e| e.push_reps_char(ch))
                    .unwrap_or(false);
                self.sync_set(target_exercise, target_set);
                if should_advance {
                    self.advance_set();
                }
            }
        }
        self.refresh_status();
    }

    pub fn backspace(&mut self) {
        if let Some(set_idx) = self.exercises.get_mut(self.selected).map(|ex| {
            match ex.focus {
                Focus::Weight => ex.backspace_weight(),
                Focus::Reps => ex.backspace_reps(),
            };
            ex.set_cursor
        }) {
            self.sync_set(self.selected, set_idx);
        }
        self.refresh_status();
    }

    fn advance_set(&mut self) {
        let Some(ex) = self.exercises.get_mut(self.selected) else {
            return;
        };
        ex.reset_input_timer();

        if ex.set_cursor + 1 < ex.sets.len() {
            ex.set_cursor += 1;
            return;
        }

        if self.selected + 1 < self.exercises.len() {
            self.selected += 1;
            if let Some(next) = self.exercises.get_mut(self.selected) {
                next.focus = Focus::Weight;
                next.set_cursor = 0;
                next.reset_input_timer();
            }
        }
    }

    pub fn sync_set(&mut self, exercise_index: usize, set_index: usize) {
        let Some(ex) = self.exercises.get(exercise_index) else {
            return;
        };
        let Some(exercise_id) = ex.exercise_id else {
            return;
        };
        let Some(set) = ex.sets.get(set_index) else {
            return;
        };
        let Some(reps) = set.reps else {
            return;
        };

        let completed_at = set.completed_at_utc().unwrap_or_else(Utc::now);
        let day = completed_at.date_naive();
        let request = SetForDayRequest {
            weight: set.weight.value as f64,
            reps: reps as i32,
            completed_at: Some(completed_at),
        };

        let _ = self.io_tx.try_send(IoRequest::SaveSet {
            exercise_id,
            set_index,
            set_number: set_index as i32 + 1,
            day,
            request,
        });
    }

    pub fn refresh_status(&mut self) {
        let Some(ex) = self.exercises.get(self.selected) else {
            self.status.exercise.clear();
            return;
        };

        let total = ex.sets.len().max(1);
        let current = (ex.set_cursor + 1).min(total);
        self.status.exercise = format!(
            "{} • {} • Set {}/{}",
            ex.name,
            ex.focus.label(),
            current,
            total
        );

        if let Some(id) = ex.exercise_id {
            self.request_graph(id);
        }
    }
}

impl AuthState {
    pub fn new_register() -> Self {
        let secret = generate_totp_secret();
        let url = build_otpauth_url("", &secret);
        Self {
            mode: AuthMode::Register,
            username: String::new(),
            password: String::new(),
            totp_code: String::new(),
            status: String::new(),
            focus: AuthField::Username,
            submitting: false,
            totp_secret: secret,
            otpauth_url: url,
        }
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        match self.focus {
            AuthField::Username => {
                self.username.push(ch);
                if self.mode == AuthMode::Register {
                    self.update_otpauth_url();
                }
            }
            AuthField::Password => self.password.push(ch),
            AuthField::Totp => self.totp_code.push(ch),
        }
    }

    pub fn backspace(&mut self) {
        match self.focus {
            AuthField::Username => {
                self.username.pop();
                if self.mode == AuthMode::Register {
                    self.update_otpauth_url();
                }
            }
            AuthField::Password => {
                self.password.pop();
            }
            AuthField::Totp => {
                self.totp_code.pop();
            }
        }
    }

    pub fn next_field(&mut self) {
        self.focus = match self.focus {
            AuthField::Username => AuthField::Password,
            AuthField::Password => AuthField::Totp,
            AuthField::Totp => AuthField::Username,
        };
    }

    pub fn prev_field(&mut self) {
        self.focus = match self.focus {
            AuthField::Username => AuthField::Totp,
            AuthField::Password => AuthField::Username,
            AuthField::Totp => AuthField::Password,
        };
    }

    pub fn regenerate_secret(&mut self) {
        self.totp_secret = generate_totp_secret();
        self.update_otpauth_url();
    }

    fn update_otpauth_url(&mut self) {
        self.otpauth_url = build_otpauth_url(&self.username, &self.totp_secret);
    }
}

fn select_plan_for_today(plans: &[PopulatedTemplate]) -> Option<&PopulatedTemplate> {
    if plans.is_empty() {
        return None;
    }
    let today = Local::now().weekday().num_days_from_monday() as i32;
    plans
        .iter()
        .find(|p| p.day_of_week == Some(today))
        .or_else(|| plans.first())
}

/// Status line content.
#[derive(Default)]
pub struct StatusLine {
    pub exercise: String,
    pub backend: String,
}

/// State for a single exercise.
#[derive(Debug, Clone)]
pub struct ExerciseState {
    pub exercise_id: Option<i64>,
    pub name: String,
    pub focus: Focus,
    pub sets: Vec<SetEntry>,
    pub set_cursor: usize,
    last_input: Option<Instant>,
}

impl ExerciseState {
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::new(None, "Back Squat", 60.0, 4),
            Self::new(None, "Bench Press", 45.0, 4),
            Self::new(None, "Bent Row", 40.0, 4),
        ]
    }

    fn new(id: Option<i64>, name: &str, weight: f32, sets: usize) -> Self {
        Self {
            exercise_id: id,
            name: name.to_string(),
            focus: Focus::Weight,
            sets: (0..sets)
                .map(|_| SetEntry::new(None, weight, false))
                .collect(),
            set_cursor: 0,
            last_input: None,
        }
    }

    pub fn from_populated(ex: &PopulatedExercise) -> Self {
        let set_count = ex.target_sets.unwrap_or(4).max(1) as usize;
        let should_prefill = ex
            .last_day_date
            .is_some_and(|d| Utc::now().signed_duration_since(d) <= ChronoDuration::days(90));

        let best_weight = if should_prefill {
            ex.last_day_sets
                .iter()
                .map(|s| s.weight as f32)
                .max_by(|a, b| a.total_cmp(b))
        } else {
            None
        };

        let weight = best_weight.unwrap_or(0.0);
        let sets = (0..set_count)
            .map(|i| {
                let reps = ex.last_day_sets.get(i).map(|s| s.reps.max(0) as u32);
                SetEntry::new(reps, weight, best_weight.is_some())
            })
            .collect();

        Self {
            exercise_id: Some(ex.exercise_id),
            name: ex.name.clone(),
            focus: Focus::Weight,
            sets,
            set_cursor: 0,
            last_input: None,
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = self.focus.toggle();
    }

    pub fn move_set_cursor(&mut self, delta: i32) {
        if self.sets.is_empty() {
            return;
        }
        let len = self.sets.len() as i32;
        let next = (self.set_cursor as i32 + delta).clamp(0, len - 1);
        self.set_cursor = next as usize;
        self.reset_input_timer();
    }

    pub fn bump_weight(&mut self, delta: f32) {
        if let Some(set) = self.sets.get_mut(self.set_cursor) {
            set.weight.bump(delta);
            let new_weight = set.weight.value;
            self.propagate_weight(new_weight);
        }
    }

    pub fn push_weight_char(&mut self, ch: char) {
        if let Some(set) = self.sets.get_mut(self.set_cursor) {
            set.weight.push_char(ch, &mut self.last_input);
            let new_weight = set.weight.value;
            self.propagate_weight(new_weight);
        }
    }

    pub fn backspace_weight(&mut self) {
        if let Some(set) = self.sets.get_mut(self.set_cursor) {
            set.weight.backspace();
        }
    }

    fn propagate_weight(&mut self, weight: f32) {
        let cursor = self.set_cursor;
        for (i, set) in self.sets.iter_mut().enumerate() {
            if i != cursor && set.reps.is_none() {
                set.weight.set_value(weight);
            }
        }
    }

    /// Push a reps character. Returns true if we should auto-advance.
    pub fn push_reps_char(&mut self, ch: char) -> bool {
        if !ch.is_ascii_digit() {
            return false;
        }

        let Some(set) = self.sets.get_mut(self.set_cursor) else {
            return false;
        };

        let now = Instant::now();
        let should_reset = self
            .last_input
            .is_none_or(|t| now.duration_since(t) > INPUT_TIMEOUT);

        if should_reset {
            set.reps_buffer.clear();
        }

        // Single digit > 2 commits immediately
        if set.reps_buffer.is_empty() && ch > '2' {
            set.reps_buffer.push(ch);
            set.apply_reps_buffer();
            self.last_input = Some(now);
            return true;
        }

        set.reps_buffer.push(ch);
        set.apply_reps_buffer();
        self.last_input = Some(now);
        false
    }

    pub fn backspace_reps(&mut self) {
        if let Some(set) = self.sets.get_mut(self.set_cursor) {
            if set.reps_buffer.pop().is_some() {
                set.apply_reps_buffer();
            } else {
                set.reps = None;
                set.started_at = None;
            }
        }
    }

    pub fn should_auto_advance(&self) -> bool {
        self.last_input.is_some_and(|t| t.elapsed() > INPUT_TIMEOUT)
    }

    pub fn reset_input_timer(&mut self) {
        self.last_input = None;
    }
}

/// Which field is focused within an exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Weight,
    Reps,
}

impl Focus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Weight => "Weight",
            Self::Reps => "Reps",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Weight => Self::Reps,
            Self::Reps => Self::Weight,
        }
    }
}

/// A single set entry.
#[derive(Debug, Clone)]
pub struct SetEntry {
    pub reps: Option<u32>,
    pub reps_buffer: String,
    pub weight: WeightEntry,
    pub started_at: Option<DateTime<Local>>,
    pub remote_id: Option<i64>,
}

impl SetEntry {
    fn new(reps: Option<u32>, weight: f32, prefill: bool) -> Self {
        Self {
            reps,
            reps_buffer: reps.map(|r| r.to_string()).unwrap_or_default(),
            weight: if prefill {
                WeightEntry::new(weight)
            } else {
                WeightEntry::empty(weight)
            },
            started_at: None,
            remote_id: None,
        }
    }

    fn apply_reps_buffer(&mut self) {
        if let Ok(v) = self.reps_buffer.parse() {
            if self.reps.is_none() {
                self.started_at = Some(Local::now());
            }
            self.reps = Some(v);
        } else {
            self.reps = None;
            self.started_at = None;
        }
    }

    pub fn reps_display(&self) -> String {
        self.reps
            .map(|r| r.to_string())
            .unwrap_or_else(|| "__".into())
    }

    pub fn completed_at_utc(&self) -> Option<DateTime<Utc>> {
        self.started_at.map(|dt| dt.with_timezone(&Utc))
    }
}

/// Weight value with input buffer.
#[derive(Debug, Clone)]
pub struct WeightEntry {
    pub value: f32,
    pub buffer: String,
}

impl WeightEntry {
    fn new(value: f32) -> Self {
        Self {
            value,
            buffer: format!("{value:.1}"),
        }
    }

    fn empty(value: f32) -> Self {
        Self {
            value,
            buffer: String::new(),
        }
    }

    pub fn set_value(&mut self, value: f32) {
        self.value = (value * 10.0).round() / 10.0;
        self.buffer = format!("{:.1}", self.value);
    }

    pub fn bump(&mut self, delta: f32) {
        let next = (self.value + delta).max(0.0);
        self.set_value(next);
    }

    pub fn display(&self) -> String {
        if self.buffer.is_empty() {
            "__".into()
        } else {
            self.buffer.clone()
        }
    }

    pub fn push_char(&mut self, ch: char, last_input: &mut Option<Instant>) {
        if !(ch.is_ascii_digit() || ch == '.') {
            return;
        }

        let now = Instant::now();
        let should_reset = last_input.is_none_or(|t| now.duration_since(t) > INPUT_TIMEOUT);

        if should_reset {
            self.buffer.clear();
        }

        self.buffer.push(ch);
        if let Ok(v) = self.buffer.parse() {
            self.value = v;
        }
        *last_input = Some(now);
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
        self.value = self.buffer.parse().unwrap_or(0.0);
    }
}

fn generate_totp_secret() -> String {
    let mut bytes = [0_u8; 20];
    OsRng.fill_bytes(&mut bytes);
    base32_encode(Alphabet::Rfc4648 { padding: false }, &bytes)
}

fn build_otpauth_url(username: &str, secret: &str) -> String {
    let label = if username.trim().is_empty() {
        "ekman".to_string()
    } else {
        format!("ekman:{}", username.trim())
    };
    let encoded_label = url_encode(&label);
    format!(
        "otpauth://totp/{label}?secret={secret}&issuer=ekman&algorithm=SHA1&digits=6&period=30",
        label = encoded_label,
        secret = secret
    )
}
