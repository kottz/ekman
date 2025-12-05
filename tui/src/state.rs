//! Application state.

use base32::{Alphabet, encode as b32_encode};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use ekman_core::{
    ActivityDay, ActivityQuery, DaySets, Graph, SetInput, Template, TemplateExercise, WorkoutSet,
};
use rand::{RngCore, rngs::OsRng};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration as StdDuration, Instant};

use crate::api::{ApiClient, Request, Response};

const INPUT_TIMEOUT: StdDuration = StdDuration::from_secs(1);
const ACTIVITY_DAYS: i64 = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Auth,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Username,
    Password,
    Totp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Weight,
    Reps,
}

pub struct App {
    pub running: bool,
    pub view: View,
    pub auth: AuthState,
    pub day: NaiveDate,
    pub exercises: Vec<ExerciseState>,
    pub graphs: Vec<Graph>,
    pub activity: Vec<ActivityDay>,
    pub selected: usize,
    pub status: String,
    api: ApiClient,
    plans: Vec<Template>,
    pending_graphs: HashSet<i64>,
    loading_sets: HashSet<(NaiveDate, i64)>,
}

pub struct AuthState {
    pub register_mode: bool,
    pub username: String,
    pub password: String,
    pub totp_code: String,
    pub totp_secret: String,
    pub status: String,
    pub field: AuthField,
    pub submitting: bool,
}

pub struct ExerciseState {
    pub id: Option<i64>,
    pub name: String,
    pub focus: Focus,
    pub sets: Vec<SetState>,
    pub cursor: usize,
    pub default_weight: f64,
    last_input: Option<Instant>,
}

pub struct SetState {
    pub id: Option<i64>,
    pub number: i32,
    pub weight: String,
    pub reps: Option<i32>,
    pub reps_buffer: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub pending: bool,
}

impl App {
    pub fn new() -> color_eyre::Result<Self> {
        let cookie_path = config_dir().join("session.cookie");
        let api = ApiClient::new(cookie_path)?;

        Ok(Self {
            running: true,
            view: View::Auth,
            auth: AuthState::new(),
            day: Utc::now().date_naive(),
            exercises: Vec::new(),
            graphs: Vec::new(),
            activity: Vec::new(),
            selected: 0,
            status: String::new(),
            api,
            plans: Vec::new(),
            pending_graphs: HashSet::new(),
            loading_sets: HashSet::new(),
        })
    }

    pub fn try_resume_session(&self) {
        self.api.send(Request::CheckSession);
    }

    pub fn poll_io(&mut self) {
        while let Ok(resp) = self.api.rx.try_recv() {
            self.handle_response(resp);
        }
    }

    pub fn tick(&mut self) {
        if self.view != View::Dashboard {
            return;
        }

        // Auto-advance after timeout
        if let Some(ex) = self.exercises.get_mut(self.selected)
            && ex.focus == Focus::Reps
            && ex.should_auto_advance()
            && ex.sets.get(ex.cursor).and_then(|s| s.reps).is_some()
        {
            self.advance_set();
        }
    }

    fn handle_response(&mut self, resp: Response) {
        match resp {
            Response::LoggedIn(result) | Response::Registered(result) => {
                self.auth.submitting = false;
                match result {
                    Ok(session) => self.on_logged_in(session.username),
                    Err(e) => self.auth.status = e,
                }
            }

            Response::SessionChecked(result) => match result {
                Ok(user) => self.on_logged_in(user.username),
                Err(e) => self.auth.status = e,
            },

            Response::Plans(result) => match result {
                Ok(plans) => {
                    self.plans = plans;
                    self.apply_day(self.day);
                }
                Err(e) => {
                    self.status = format!("Load error: {e}");
                    self.exercises.clear();
                }
            },

            Response::Graph(id, result) => {
                self.pending_graphs.remove(&id);
                match result {
                    Ok(graph) => {
                        if let Some(g) = self.graphs.iter_mut().find(|g| g.exercise_id == id) {
                            *g = graph;
                        } else {
                            self.graphs.push(graph);
                        }
                    }
                    Err(e) => self.status = format!("Graph error: {e}"),
                }
            }

            Response::Activity(result) => match result {
                Ok(activity) => self.activity = activity.days,
                Err(e) => self.status = format!("Activity error: {e}"),
            },

            Response::SetsLoaded {
                exercise_id,
                day,
                result,
            } => {
                self.loading_sets.remove(&(day, exercise_id));
                if day != self.day {
                    return;
                }
                match result {
                    Ok(sets) => {
                        if let Some(ex) = self
                            .exercises
                            .iter_mut()
                            .find(|e| e.id == Some(exercise_id))
                        {
                            ex.apply_server_sets(sets);
                        }
                    }
                    Err(e) => self.status = format!("Load sets error: {e}"),
                }
            }

            Response::SetSaved {
                exercise_id,
                day,
                set_number,
                result,
            } => {
                if day != self.day {
                    return;
                }
                match result {
                    Ok(saved) => {
                        if let Some(ex) = self
                            .exercises
                            .iter_mut()
                            .find(|e| e.id == Some(exercise_id))
                        {
                            ex.apply_saved_set(&saved);
                            self.status = format!("Saved set {}", set_number);
                        }
                        self.request_activity();
                    }
                    Err(e) => {
                        self.status = format!("Save error: {e}");
                        self.request_sets_for(exercise_id);
                    }
                }
            }

            Response::SetDeleted {
                exercise_id,
                day,
                set_number,
                result,
            } => {
                if day != self.day {
                    return;
                }
                match result {
                    Ok(()) => {
                        self.status = format!("Deleted set {}", set_number);
                        self.request_sets_for(exercise_id);
                        self.request_activity();
                    }
                    Err(e) => self.status = format!("Delete error: {e}"),
                }
            }
        }
    }

    fn on_logged_in(&mut self, username: String) {
        self.view = View::Dashboard;
        self.auth.submitting = false;
        self.status = format!("Signed in as {username}");
        self.api.save_cookie();
        self.api.send(Request::LoadPlans);
        self.request_activity();
    }

    fn request_activity(&self) {
        let end = Utc::now();
        let start = end - Duration::days(ACTIVITY_DAYS - 1);
        self.api.send(Request::LoadActivity(ActivityQuery {
            start: Some(start),
            end: Some(end),
        }));
    }

    fn request_graph(&mut self, id: i64) {
        if !self.pending_graphs.contains(&id) && !self.graphs.iter().any(|g| g.exercise_id == id) {
            self.pending_graphs.insert(id);
            self.api.send(Request::LoadGraph(id));
        }
    }

    fn request_sets_for(&mut self, exercise_id: i64) {
        let key = (self.day, exercise_id);
        if !self.loading_sets.contains(&key) {
            self.loading_sets.insert(key);
            self.api.send(Request::LoadSets {
                day: self.day,
                exercise_id,
            });
        }
    }

    fn request_all_sets(&mut self) {
        for id in self
            .exercises
            .iter()
            .filter_map(|e| e.id)
            .collect::<Vec<_>>()
        {
            self.request_sets_for(id);
        }
    }

    fn request_all_graphs(&mut self) {
        for id in self
            .exercises
            .iter()
            .filter_map(|e| e.id)
            .collect::<Vec<_>>()
        {
            self.request_graph(id);
        }
    }

    fn apply_day(&mut self, day: NaiveDate) {
        self.day = day;
        self.selected = 0;
        self.loading_sets.clear();

        // Find matching plan
        let weekday = day.weekday().num_days_from_monday() as i32;
        let plan = self
            .plans
            .iter()
            .find(|p| p.day_of_week == Some(weekday))
            .or_else(|| self.plans.first());

        match plan {
            Some(p) => {
                self.status = format!("{} • {}", p.name, day.format("%a %b %e"));
                self.exercises = p
                    .exercises
                    .iter()
                    .map(ExerciseState::from_template)
                    .collect();
            }
            None => {
                self.status = "No plans available".into();
                self.exercises.clear();
            }
        }

        // Clean up stale graph data
        let ids: HashSet<_> = self.exercises.iter().filter_map(|e| e.id).collect();
        self.graphs.retain(|g| ids.contains(&g.exercise_id));
        self.pending_graphs.retain(|id| ids.contains(id));

        self.request_all_graphs();
        self.request_all_sets();
    }

    // Auth actions

    pub fn set_auth_mode(&mut self, register: bool) {
        if self.auth.register_mode == register {
            return;
        }
        self.auth.register_mode = register;
        self.auth.totp_code.clear();
        self.auth.status.clear();
        self.auth.field = AuthField::Username;
        if register {
            self.auth.regenerate_secret();
        }
    }

    pub fn submit_auth(&mut self) {
        if self.auth.submitting {
            return;
        }

        if self.auth.username.trim().is_empty() {
            self.auth.status = "Username required".into();
            return;
        }
        if self.auth.password.is_empty() {
            self.auth.status = "Password required".into();
            return;
        }
        if self.auth.totp_code.is_empty() {
            self.auth.status = "TOTP code required".into();
            return;
        }

        self.auth.submitting = true;
        self.auth.status = if self.auth.register_mode {
            "Registering..."
        } else {
            "Signing in..."
        }
        .into();

        if self.auth.register_mode {
            self.api.send(Request::Register {
                username: self.auth.username.clone(),
                password: self.auth.password.clone(),
                totp_secret: self.auth.totp_secret.clone(),
                totp_code: self.auth.totp_code.clone(),
            });
        } else {
            self.api.send(Request::Login {
                username: self.auth.username.clone(),
                password: self.auth.password.clone(),
                totp: self.auth.totp_code.clone(),
            });
        }
    }

    // Day navigation

    pub fn move_day(&mut self, delta: i64) {
        if let Some(next) = self.day.checked_add_signed(Duration::days(delta)) {
            self.apply_day(next);
        }
    }

    pub fn jump_to_today(&mut self) {
        let today = Utc::now().date_naive();
        if self.day != today {
            self.apply_day(today);
        }
    }

    // Exercise navigation

    pub fn select_exercise(&mut self, delta: i32) {
        if self.exercises.is_empty() {
            return;
        }

        // Trim empty sets from current
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            ex.trim_empty_trailing();
        }

        let len = self.exercises.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1);
        self.selected = next as usize;

        if let Some(ex) = self.exercises.get_mut(self.selected) {
            ex.reset_timer();
        }

        self.request_current_sets();
    }

    pub fn toggle_focus(&mut self) {
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            ex.focus = match ex.focus {
                Focus::Weight => Focus::Reps,
                Focus::Reps => Focus::Weight,
            };
            ex.reset_timer();
        }
    }

    pub fn move_set_cursor(&mut self, delta: i32) {
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            let len = ex.sets.len() as i32;
            let next = (ex.cursor as i32 + delta).clamp(0, len.saturating_sub(1));
            ex.cursor = next as usize;
            ex.reset_timer();
        }
    }

    pub fn tab_next(&mut self) {
        if self.exercises.is_empty() {
            return;
        }

        if let Some(ex) = self.exercises.get_mut(self.selected) {
            match ex.focus {
                Focus::Weight => {
                    ex.focus = Focus::Reps;
                    ex.reset_timer();
                    return;
                }
                Focus::Reps if ex.cursor + 1 < ex.sets.len() => {
                    ex.cursor += 1;
                    ex.focus = Focus::Weight;
                    ex.reset_timer();
                    return;
                }
                Focus::Reps => {}
            }
        }

        // Move to next exercise
        if self.selected + 1 < self.exercises.len() {
            if let Some(ex) = self.exercises.get_mut(self.selected) {
                ex.trim_empty_trailing();
            }
            self.selected += 1;
            if let Some(ex) = self.exercises.get_mut(self.selected) {
                ex.cursor = 0;
                ex.focus = Focus::Weight;
                ex.reset_timer();
            }
            self.request_current_sets();
        }
    }

    pub fn tab_prev(&mut self) {
        if self.exercises.is_empty() {
            return;
        }

        if let Some(ex) = self.exercises.get_mut(self.selected) {
            match ex.focus {
                Focus::Reps => {
                    ex.focus = Focus::Weight;
                    ex.reset_timer();
                    return;
                }
                Focus::Weight if ex.cursor > 0 => {
                    ex.cursor -= 1;
                    ex.focus = Focus::Reps;
                    ex.reset_timer();
                    return;
                }
                Focus::Weight => {}
            }
        }

        // Move to prev exercise
        if self.selected > 0 {
            if let Some(ex) = self.exercises.get_mut(self.selected) {
                ex.trim_empty_trailing();
            }
            self.selected -= 1;
            if let Some(ex) = self.exercises.get_mut(self.selected) {
                ex.cursor = ex.sets.len().saturating_sub(1);
                ex.focus = Focus::Reps;
                ex.reset_timer();
            }
            self.request_current_sets();
        }
    }

    // Input

    pub fn input_char(&mut self, ch: char) {
        let Some(ex) = self.exercises.get_mut(self.selected) else {
            return;
        };

        match ex.focus {
            Focus::Weight => {
                ex.push_weight_char(ch);
                self.sync_current_set();
            }
            Focus::Reps => {
                // Check auto-advance before input
                if ex.should_auto_advance() {
                    self.advance_set();
                }

                let should_advance = {
                    let ex = self.exercises.get_mut(self.selected).unwrap();
                    ex.push_reps_char(ch)
                };

                self.sync_current_set();

                if should_advance {
                    self.advance_set();
                }
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            match ex.focus {
                Focus::Weight => ex.backspace_weight(),
                Focus::Reps => ex.backspace_reps(),
            }
            self.sync_current_set();
        }
    }

    pub fn bump_weight(&mut self, delta: f64) {
        if let Some(ex) = self.exercises.get_mut(self.selected) {
            ex.bump_weight(delta);
            self.sync_current_set();
        }
    }

    pub fn delete_current_set(&mut self) {
        let Some(ex) = self.exercises.get_mut(self.selected) else {
            return;
        };
        let Some(exercise_id) = ex.id else {
            return;
        };

        if ex.sets.is_empty() {
            return;
        }

        let set_number = ex.sets.get(ex.cursor).map(|s| s.number).unwrap_or(1);
        let removed = ex.sets.remove(ex.cursor);

        // Ensure at least one set
        if ex.sets.is_empty() {
            ex.sets.push(SetState::empty(1, ex.default_weight));
        }

        // Renumber
        for (i, s) in ex.sets.iter_mut().enumerate() {
            s.number = i as i32 + 1;
        }

        ex.cursor = ex.cursor.min(ex.sets.len().saturating_sub(1));

        if removed.id.is_some() {
            self.status = format!("Deleting set {}...", set_number);
            self.api.send(Request::DeleteSet {
                day: self.day,
                exercise_id,
                set_number,
            });
        }
    }

    fn advance_set(&mut self) {
        let Some(ex) = self.exercises.get_mut(self.selected) else {
            return;
        };
        ex.reset_timer();

        if ex.cursor + 1 < ex.sets.len() {
            ex.cursor += 1;
            ex.focus = Focus::Reps;
            return;
        }

        // Append new set
        let weight = ex
            .sets
            .last()
            .map(|s| parse_weight(&s.weight))
            .unwrap_or(ex.default_weight);
        ex.default_weight = weight;
        ex.sets
            .push(SetState::empty(ex.sets.len() as i32 + 1, weight));
        ex.cursor = ex.sets.len() - 1;
        ex.focus = Focus::Reps;
    }

    fn sync_current_set(&mut self) {
        let Some(ex) = self.exercises.get_mut(self.selected) else {
            return;
        };
        let Some(exercise_id) = ex.id else {
            return;
        };

        let Some(set) = ex.sets.get_mut(ex.cursor) else {
            return;
        };

        let Some(reps) = set.reps else {
            return;
        };

        let weight = parse_weight(&set.weight);
        let completed_at = set.completed_at.unwrap_or_else(|| {
            let noon = self.day.and_hms_opt(12, 0, 0).unwrap();
            DateTime::from_naive_utc_and_offset(noon, Utc)
        });

        set.pending = true;

        self.api.send(Request::SaveSet {
            day: self.day,
            exercise_id,
            set_number: set.number,
            input: SetInput {
                weight,
                reps,
                completed_at: Some(completed_at),
            },
        });
    }

    fn request_current_sets(&mut self) {
        if let Some(id) = self.exercises.get(self.selected).and_then(|e| e.id) {
            self.request_sets_for(id);
        }
    }

    pub fn current_plan_name(&self) -> Option<&str> {
        let weekday = self.day.weekday().num_days_from_monday() as i32;
        self.plans
            .iter()
            .find(|p| p.day_of_week == Some(weekday))
            .or_else(|| self.plans.first())
            .map(|p| p.name.as_str())
    }
}

impl AuthState {
    pub fn new() -> Self {
        let secret = generate_totp_secret();
        Self {
            register_mode: true,
            username: String::new(),
            password: String::new(),
            totp_code: String::new(),
            totp_secret: secret,
            status: String::new(),
            field: AuthField::Username,
            submitting: false,
        }
    }

    pub fn otpauth_url(&self) -> String {
        let label = if self.username.trim().is_empty() {
            "ekman".to_string()
        } else {
            format!("ekman:{}", self.username.trim())
        };
        format!(
            "otpauth://totp/{}?secret={}&issuer=ekman&algorithm=SHA1&digits=6&period=30",
            url_encode(&label),
            self.totp_secret
        )
    }

    pub fn next_field(&mut self) {
        self.field = match self.field {
            AuthField::Username => AuthField::Password,
            AuthField::Password => AuthField::Totp,
            AuthField::Totp => AuthField::Username,
        };
    }

    pub fn prev_field(&mut self) {
        self.field = match self.field {
            AuthField::Username => AuthField::Totp,
            AuthField::Password => AuthField::Username,
            AuthField::Totp => AuthField::Password,
        };
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        match self.field {
            AuthField::Username => self.username.push(ch),
            AuthField::Password => self.password.push(ch),
            AuthField::Totp => self.totp_code.push(ch),
        }
    }

    pub fn backspace(&mut self) {
        match self.field {
            AuthField::Username => {
                self.username.pop();
            }
            AuthField::Password => {
                self.password.pop();
            }
            AuthField::Totp => {
                self.totp_code.pop();
            }
        }
    }

    pub fn regenerate_secret(&mut self) {
        self.totp_secret = generate_totp_secret();
    }
}

impl ExerciseState {
    pub fn from_template(ex: &TemplateExercise) -> Self {
        let weight = ex
            .last_session
            .as_ref()
            .filter(|s| Utc::now().signed_duration_since(s.date) <= Duration::days(90))
            .and_then(|s| {
                s.sets
                    .iter()
                    .map(|set| set.weight)
                    .max_by(|a, b| a.total_cmp(b))
            })
            .unwrap_or(0.0);

        Self {
            id: Some(ex.exercise_id),
            name: ex.name.clone(),
            focus: Focus::Weight,
            sets: vec![SetState::empty(1, weight)],
            cursor: 0,
            default_weight: weight,
            last_input: None,
        }
    }

    pub fn should_auto_advance(&self) -> bool {
        self.last_input.is_some_and(|t| t.elapsed() > INPUT_TIMEOUT)
    }

    pub fn reset_timer(&mut self) {
        self.last_input = None;
    }

    pub fn push_weight_char(&mut self, ch: char) {
        if !(ch.is_ascii_digit() || ch == '.') {
            return;
        }

        let should_reset = self.last_input.is_none_or(|t| t.elapsed() > INPUT_TIMEOUT);
        if should_reset && let Some(set) = self.sets.get_mut(self.cursor) {
            set.weight.clear();
        }

        if let Some(set) = self.sets.get_mut(self.cursor) {
            set.weight.push(ch);
            let w = parse_weight(&set.weight);
            self.propagate_weight(w);
            self.default_weight = w;
        }

        self.last_input = Some(Instant::now());
    }

    pub fn backspace_weight(&mut self) {
        if let Some(set) = self.sets.get_mut(self.cursor) {
            set.weight.pop();
            self.default_weight = parse_weight(&set.weight);
        }
    }

    /// Returns true if we should auto-advance.
    pub fn push_reps_char(&mut self, ch: char) -> bool {
        if !ch.is_ascii_digit() {
            return false;
        }

        let Some(set) = self.sets.get_mut(self.cursor) else {
            return false;
        };

        let should_reset = self.last_input.is_none_or(|t| t.elapsed() > INPUT_TIMEOUT);
        if should_reset {
            set.reps_buffer.clear();
        }

        // Single digit > 2 commits immediately
        if set.reps_buffer.is_empty() && ch > '2' {
            set.reps_buffer.push(ch);
            set.apply_reps_buffer();
            self.last_input = Some(Instant::now());
            return true;
        }

        set.reps_buffer.push(ch);
        set.apply_reps_buffer();
        self.last_input = Some(Instant::now());
        false
    }

    pub fn backspace_reps(&mut self) {
        if let Some(set) = self.sets.get_mut(self.cursor) {
            if set.reps_buffer.pop().is_some() {
                set.apply_reps_buffer();
            } else {
                set.reps = None;
                set.completed_at = None;
            }
        }
    }

    pub fn bump_weight(&mut self, delta: f64) {
        if let Some(set) = self.sets.get_mut(self.cursor) {
            let w = (parse_weight(&set.weight) + delta).max(0.0);
            set.weight = format!("{w:.1}");
            self.propagate_weight(w);
            self.default_weight = w;
        }
    }

    fn propagate_weight(&mut self, weight: f64) {
        let cursor = self.cursor;
        for (i, set) in self.sets.iter_mut().enumerate() {
            if i != cursor && set.reps.is_none() {
                set.weight = format!("{weight:.1}");
            }
        }
    }

    pub fn trim_empty_trailing(&mut self) {
        while self.sets.len() > 1 && self.sets.last().is_some_and(|s| s.is_empty()) {
            self.sets.pop();
        }
        self.cursor = self.cursor.min(self.sets.len().saturating_sub(1));
    }

    pub fn visible_len(&self, selected: bool) -> usize {
        if selected {
            return self.sets.len().max(1);
        }
        let trailing = self.sets.iter().rev().take_while(|s| s.is_empty()).count();
        self.sets.len().saturating_sub(trailing).max(1)
    }

    pub fn apply_server_sets(&mut self, data: DaySets) {
        if data.sets.is_empty() {
            self.sets = vec![SetState::empty(1, self.default_weight)];
        } else {
            self.sets = data.sets.into_iter().map(SetState::from_server).collect();
        }
        self.cursor = self.cursor.min(self.sets.len().saturating_sub(1));
        if let Some(last) = self.sets.last() {
            self.default_weight = parse_weight(&last.weight);
        }
    }

    pub fn apply_saved_set(&mut self, saved: &WorkoutSet) {
        if let Some(set) = self.sets.iter_mut().find(|s| s.number == saved.set_number) {
            set.id = Some(saved.id);
            set.reps = Some(saved.reps);
            set.reps_buffer = saved.reps.to_string();
            set.completed_at = Some(saved.completed_at);
            set.pending = false;
            // Only update weight if not actively editing
            if self.last_input.is_none_or(|t| t.elapsed() > INPUT_TIMEOUT) {
                set.weight = format!("{:.1}", saved.weight);
                self.default_weight = saved.weight;
            }
        } else {
            self.sets.push(SetState::from_server(saved.clone()));
            self.sets.sort_by_key(|s| s.number);
        }
    }
}

impl SetState {
    pub fn empty(number: i32, weight: f64) -> Self {
        Self {
            id: None,
            number,
            weight: if weight > 0.0 {
                format!("{weight:.1}")
            } else {
                String::new()
            },
            reps: None,
            reps_buffer: String::new(),
            completed_at: None,
            pending: false,
        }
    }

    pub fn from_server(s: WorkoutSet) -> Self {
        Self {
            id: Some(s.id),
            number: s.set_number,
            weight: format!("{:.1}", s.weight),
            reps: Some(s.reps),
            reps_buffer: s.reps.to_string(),
            completed_at: Some(s.completed_at),
            pending: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.reps.is_none() && self.completed_at.is_none() && !self.pending
    }

    fn apply_reps_buffer(&mut self) {
        if let Ok(r) = self.reps_buffer.parse() {
            self.reps = Some(r);
            if self.completed_at.is_none() {
                self.completed_at = Some(Utc::now());
            }
        } else {
            self.reps = None;
            self.completed_at = None;
        }
    }

    pub fn weight_display(&self) -> String {
        if self.weight.is_empty() {
            "__".into()
        } else {
            self.weight.clone()
        }
    }

    pub fn reps_display(&self) -> String {
        self.reps
            .map(|r| r.to_string())
            .unwrap_or_else(|| "__".into())
    }

    pub fn completed_local(&self) -> Option<DateTime<Local>> {
        self.completed_at.map(|dt| dt.with_timezone(&Local))
    }
}

fn parse_weight(s: &str) -> f64 {
    s.parse().unwrap_or(0.0)
}

fn generate_totp_secret() -> String {
    let mut bytes = [0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    b32_encode(Alphabet::Rfc4648 { padding: false }, &bytes)
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("ekman"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(ch),
            _ => {
                for b in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    result
}
