// ============================================================================
// Ekman - Workout Tracker Web App
// ============================================================================

const API_BASE = "http://localhost:3000"; //window.location.origin;

// ============================================================================
// State
// ============================================================================

const state = {
    user: null,
    currentDate: new Date(),
    plans: [],
    exercises: [],
    activity: [],
    todaySets: {},  // { exerciseId: [sets] }
    selectedPlanDay: null,
    editingExercise: null,
    editingSet: null,
    totpSecret: '',
};

// ============================================================================
// API
// ============================================================================

async function api(method, path, body = null) {
    const options = {
        method,
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
    };
    if (body) {
        options.body = JSON.stringify(body);
    }
    const res = await fetch(`${API_BASE}${path}`, options);
    if (!res.ok) {
        const text = await res.text();
        throw new Error(text || res.statusText);
    }
    if (res.status === 204) return null;
    return res.json();
}

// ============================================================================
// Auth
// ============================================================================

function generateTotpSecret() {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
    let secret = '';
    const array = new Uint8Array(20);
    crypto.getRandomValues(array);
    for (let i = 0; i < 20; i++) {
        secret += chars[array[i] % 32];
    }
    return secret;
}

function generateQRCode(secret, username) {
    const label = username ? `ekman:${username}` : 'ekman';
    const url = `otpauth://totp/${encodeURIComponent(label)}?secret=${secret}&issuer=ekman&algorithm=SHA1&digits=6&period=30`;
    
    // Simple QR code using a library-free approach via API
    const canvas = document.getElementById('qr-code');
    const size = 150;
    canvas.width = size;
    canvas.height = size;
    
    // Use QR code API service
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => {
        const ctx = canvas.getContext('2d');
        ctx.fillStyle = 'white';
        ctx.fillRect(0, 0, size, size);
        ctx.drawImage(img, 0, 0, size, size);
    };
    img.src = `https://api.qrserver.com/v1/create-qr-code/?size=${size}x${size}&data=${encodeURIComponent(url)}`;
}

async function checkSession() {
    try {
        const user = await api('GET', '/api/auth/me');
        state.user = user;
        showMainView();
        loadAllData();
    } catch (e) {
        showAuthView();
    }
}

async function login(username, password, totp) {
    const session = await api('POST', '/api/auth/login', { username, password, totp });
    state.user = { username: session.username };
    showMainView();
    loadAllData();
}

async function register(username, password, totpSecret, totpCode) {
    const session = await api('POST', '/api/auth/register', {
        username,
        password,
        totp_secret: totpSecret,
        totp_code: totpCode,
    });
    state.user = { username: session.username };
    showMainView();
    loadAllData();
}

// ============================================================================
// Data Loading
// ============================================================================

async function loadAllData() {
    await Promise.all([
        loadPlans(),
        loadExercises(),
        loadActivity(),
    ]);
    loadDaySets();
}

async function loadPlans() {
    try {
        state.plans = await api('GET', '/api/plans/daily');
        renderWeekdayList();
    } catch (e) {
        showStatus('Failed to load plans', 'error');
    }
}

async function loadExercises() {
    try {
        state.exercises = await api('GET', '/api/exercises');
        renderAllExercises();
    } catch (e) {
        showStatus('Failed to load exercises', 'error');
    }
}

async function loadActivity() {
    try {
        const end = new Date();
        const start = new Date();
        start.setDate(start.getDate() - 20);
        const activity = await api('GET', `/api/activity/days?start=${start.toISOString()}&end=${end.toISOString()}`);
        state.activity = activity.days || [];
        renderActivityBar();
    } catch (e) {
        console.error('Failed to load activity:', e);
    }
}

async function loadDaySets() {
    const dateStr = formatDate(state.currentDate);
    const plan = getPlanForDate(state.currentDate);
    
    if (!plan || plan.exercises.length === 0) {
        state.todaySets = {};
        renderExercisesList();
        return;
    }
    
    state.todaySets = {};
    
    for (const ex of plan.exercises) {
        try {
            const data = await api('GET', `/api/days/${dateStr}/exercises/${ex.exercise_id}/sets`);
            state.todaySets[ex.exercise_id] = data.sets || [];
        } catch (e) {
            state.todaySets[ex.exercise_id] = [];
        }
    }
    
    renderExercisesList();
}

// ============================================================================
// Plans
// ============================================================================

function getPlanForDate(date) {
    const weekday = (date.getDay() + 6) % 7; // Convert to Mon=0
    return state.plans.find(p => p.day_of_week === weekday);
}

async function createPlan(name, dayOfWeek) {
    return api('POST', '/api/plans', { name, day_of_week: dayOfWeek });
}

async function addExerciseToPlan(templateId, exerciseId) {
    await api('POST', `/api/plans/${templateId}/exercises`, { exercise_id: exerciseId });
    await loadPlans();
    showStatus('Exercise added to plan', 'success');
}

async function removeExerciseFromPlan(templateId, exerciseId) {
    await api('DELETE', `/api/plans/${templateId}/exercises/${exerciseId}`);
    await loadPlans();
    showStatus('Exercise removed from plan', 'success');
}

// ============================================================================
// Exercises
// ============================================================================

async function createExercise(name) {
    const exercise = await api('POST', '/api/exercises', { name });
    state.exercises.push(exercise);
    renderAllExercises();
    showStatus('Exercise created', 'success');
    return exercise;
}

async function updateExercise(id, updates) {
    const exercise = await api('PATCH', `/api/exercises/${id}`, updates);
    const idx = state.exercises.findIndex(e => e.id === id);
    if (idx >= 0) state.exercises[idx] = exercise;
    renderAllExercises();
    showStatus('Exercise updated', 'success');
}

// ============================================================================
// Sets
// ============================================================================

async function saveSet(exerciseId, setNumber, weight, reps) {
    const dateStr = formatDate(state.currentDate);
    const set = await api('PUT', `/api/days/${dateStr}/exercises/${exerciseId}/sets/${setNumber}`, {
        weight: weight || null,
        reps: reps || null,
        completed_at: new Date().toISOString(),
    });
    
    // Update local state
    if (!state.todaySets[exerciseId]) state.todaySets[exerciseId] = [];
    const idx = state.todaySets[exerciseId].findIndex(s => s.set_number === setNumber);
    if (idx >= 0) {
        state.todaySets[exerciseId][idx] = set;
    } else {
        state.todaySets[exerciseId].push(set);
        state.todaySets[exerciseId].sort((a, b) => a.set_number - b.set_number);
    }
    
    renderExercisesList();
    loadActivity();
    showStatus('Set saved', 'success');
}

async function deleteSet(exerciseId, setNumber) {
    const dateStr = formatDate(state.currentDate);
    await api('DELETE', `/api/days/${dateStr}/exercises/${exerciseId}/sets/${setNumber}`);
    
    // Update local state
    if (state.todaySets[exerciseId]) {
        state.todaySets[exerciseId] = state.todaySets[exerciseId].filter(s => s.set_number !== setNumber);
    }
    
    renderExercisesList();
    loadActivity();
    showStatus('Set deleted', 'success');
}

// ============================================================================
// UI Helpers
// ============================================================================

function formatDate(date) {
    return date.toISOString().split('T')[0];
}

function formatTime(isoString) {
    if (!isoString) return '';
    const d = new Date(isoString);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

function getWeekdayName(date) {
    return WEEKDAYS[(date.getDay() + 6) % 7];
}

function showStatus(message, type = 'info') {
    const bar = document.getElementById('status-bar');
    bar.textContent = message;
    bar.className = `status-bar visible ${type}`;
    setTimeout(() => {
        bar.classList.remove('visible');
    }, 3000);
}

// ============================================================================
// View Management
// ============================================================================

function showAuthView() {
    document.getElementById('auth-view').classList.remove('hidden');
    document.getElementById('main-view').classList.add('hidden');
}

function showMainView() {
    document.getElementById('auth-view').classList.add('hidden');
    document.getElementById('main-view').classList.remove('hidden');
    updateDayDisplay();
}

function switchTab(tabName) {
    // Update nav buttons
    document.querySelectorAll('.nav-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.view === tabName);
    });
    
    // Update tab content
    document.querySelectorAll('.tab-content').forEach(tab => {
        tab.classList.add('hidden');
    });
    document.getElementById(`${tabName}-tab`).classList.remove('hidden');
    
    // Reset plan detail view when switching away
    if (tabName !== 'plans') {
        document.getElementById('weekday-list').classList.remove('hidden');
        document.getElementById('plan-detail').classList.add('hidden');
    }
}

// ============================================================================
// Render Functions
// ============================================================================

function updateDayDisplay() {
    document.getElementById('current-day').textContent = getWeekdayName(state.currentDate);
    document.getElementById('current-date').textContent = formatDate(state.currentDate);
}

function renderActivityBar() {
    const container = document.getElementById('activity-bar');
    container.innerHTML = '';
    
    const today = formatDate(new Date());
    const activitySet = new Set(state.activity.map(d => d.day));
    
    // Show last 21 days
    for (let i = 20; i >= 0; i--) {
        const date = new Date();
        date.setDate(date.getDate() - i);
        const dateStr = formatDate(date);
        
        const dot = document.createElement('div');
        dot.className = 'activity-day';
        if (activitySet.has(dateStr)) dot.classList.add('has-activity');
        if (dateStr === today) dot.classList.add('today');
        dot.title = dateStr;
        container.appendChild(dot);
    }
}

function renderExercisesList() {
    const container = document.getElementById('exercises-list');
    const emptyState = document.getElementById('no-exercises');
    const plan = getPlanForDate(state.currentDate);
    
    if (!plan || plan.exercises.length === 0) {
        container.innerHTML = '';
        emptyState.classList.remove('hidden');
        return;
    }
    
    emptyState.classList.add('hidden');
    container.innerHTML = plan.exercises.map(ex => {
        const sets = state.todaySets[ex.exercise_id] || [];
        const maxSet = sets.length > 0 ? Math.max(...sets.map(s => s.set_number)) : 0;
        const displaySets = Math.max(ex.target_sets || 3, maxSet + 1);
        
        return `
            <div class="exercise-card" data-exercise-id="${ex.exercise_id}">
                <div class="exercise-header">
                    <span class="exercise-name">${escapeHtml(ex.name)}</span>
                    <span class="exercise-meta">${sets.filter(s => s.reps).length}/${displaySets} sets</span>
                </div>
                <div class="sets-container">
                    ${Array.from({ length: displaySets }, (_, i) => {
                        const setNum = i + 1;
                        const set = sets.find(s => s.set_number === setNum);
                        const completed = set && set.reps;
                        return `
                            <div class="set-item ${completed ? 'completed' : ''}" 
                                 data-set="${setNum}" data-exercise="${ex.exercise_id}">
                                <span class="set-weight">${set?.weight ? set.weight + ' kg' : '-'}</span>
                                <span class="set-reps">${set?.reps ? set.reps + ' reps' : '-'}</span>
                                ${set?.completed_at ? `<span class="set-time">${formatTime(set.completed_at)}</span>` : ''}
                            </div>
                        `;
                    }).join('')}
                    <button class="add-set-btn" data-exercise="${ex.exercise_id}" data-set="${displaySets + 1}">+</button>
                </div>
            </div>
        `;
    }).join('');
    
    // Add click handlers for sets
    container.querySelectorAll('.set-item, .add-set-btn').forEach(el => {
        el.addEventListener('click', () => {
            const exerciseId = parseInt(el.dataset.exercise);
            const setNumber = parseInt(el.dataset.set);
            const exercise = plan.exercises.find(e => e.exercise_id === exerciseId);
            openSetModal(exerciseId, exercise?.name || 'Exercise', setNumber);
        });
    });
}

function renderWeekdayList() {
    const container = document.getElementById('weekday-list');
    container.innerHTML = WEEKDAYS.map((day, i) => {
        const plan = state.plans.find(p => p.day_of_week === i);
        const exerciseCount = plan?.exercises?.length || 0;
        return `
            <div class="weekday-item" data-day="${i}">
                <div class="weekday-info">
                    <span class="weekday-name">${day}</span>
                    <span class="weekday-exercises">${exerciseCount} exercise${exerciseCount !== 1 ? 's' : ''}</span>
                </div>
                <span class="weekday-arrow">▶</span>
            </div>
        `;
    }).join('');
    
    container.querySelectorAll('.weekday-item').forEach(el => {
        el.addEventListener('click', () => {
            const day = parseInt(el.dataset.day);
            showPlanDetail(day);
        });
    });
}

function showPlanDetail(dayIndex) {
    state.selectedPlanDay = dayIndex;
    
    document.getElementById('weekday-list').classList.add('hidden');
    document.getElementById('plan-detail').classList.remove('hidden');
    document.getElementById('plan-day-name').textContent = WEEKDAYS[dayIndex];
    
    renderPlanExercises();
}

function renderPlanExercises() {
    const container = document.getElementById('plan-exercises');
    const plan = state.plans.find(p => p.day_of_week === state.selectedPlanDay);
    
    if (!plan || plan.exercises.length === 0) {
        container.innerHTML = '<p class="empty-state">No exercises yet</p>';
        return;
    }
    
    container.innerHTML = plan.exercises.map(ex => `
        <div class="plan-exercise-item" data-exercise-id="${ex.exercise_id}">
            <span class="plan-exercise-name">${escapeHtml(ex.name)}</span>
            <button class="remove-exercise-btn" data-template="${plan.id}" data-exercise="${ex.exercise_id}">×</button>
        </div>
    `).join('');
    
    container.querySelectorAll('.remove-exercise-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const templateId = parseInt(btn.dataset.template);
            const exerciseId = parseInt(btn.dataset.exercise);
            await removeExerciseFromPlan(templateId, exerciseId);
            renderPlanExercises();
        });
    });
}

function renderAllExercises() {
    const container = document.getElementById('all-exercises-list');
    const showArchived = document.getElementById('show-archived').checked;
    
    const filtered = state.exercises.filter(e => showArchived || !e.archived);
    
    if (filtered.length === 0) {
        container.innerHTML = '<p class="empty-state">No exercises yet</p>';
        return;
    }
    
    container.innerHTML = filtered.map(ex => `
        <div class="all-exercise-item ${ex.archived ? 'archived' : ''}" data-id="${ex.id}">
            <div class="exercise-item-info">
                <span class="exercise-item-name">${escapeHtml(ex.name)}</span>
                <span class="exercise-item-status">${ex.archived ? 'Archived' : 'Active'} • ${ex.owner === 'user' ? 'You' : 'System'}</span>
            </div>
            <div class="exercise-item-actions">
                <button class="action-btn edit" data-id="${ex.id}" title="Rename">✎</button>
                <button class="action-btn archive" data-id="${ex.id}" title="${ex.archived ? 'Unarchive' : 'Archive'}">${ex.archived ? '↩' : '✕'}</button>
            </div>
        </div>
    `).join('');
    
    container.querySelectorAll('.action-btn.edit').forEach(btn => {
        btn.addEventListener('click', () => {
            const id = parseInt(btn.dataset.id);
            const ex = state.exercises.find(e => e.id === id);
            if (ex) openExerciseModal(ex);
        });
    });
    
    container.querySelectorAll('.action-btn.archive').forEach(btn => {
        btn.addEventListener('click', async () => {
            const id = parseInt(btn.dataset.id);
            const ex = state.exercises.find(e => e.id === id);
            if (ex) {
                await updateExercise(id, { archived: !ex.archived });
            }
        });
    });
}

function renderSearchResults(query) {
    const container = document.getElementById('search-results');
    const filtered = state.exercises
        .filter(e => !e.archived)
        .filter(e => !query || e.name.toLowerCase().includes(query.toLowerCase()));
    
    container.innerHTML = filtered.map(ex => `
        <div class="search-result-item" data-id="${ex.id}">${escapeHtml(ex.name)}</div>
    `).join('');
    
    container.querySelectorAll('.search-result-item').forEach(el => {
        el.addEventListener('click', async () => {
            const exerciseId = parseInt(el.dataset.id);
            await addExerciseToPlanDay(exerciseId);
        });
    });
}

async function addExerciseToPlanDay(exerciseId) {
    let plan = state.plans.find(p => p.day_of_week === state.selectedPlanDay);
    
    if (!plan) {
        // Create plan first
        const dayName = WEEKDAYS[state.selectedPlanDay];
        plan = await createPlan(dayName, state.selectedPlanDay);
        await loadPlans();
        plan = state.plans.find(p => p.day_of_week === state.selectedPlanDay);
    }
    
    await addExerciseToPlan(plan.id, exerciseId);
    closeModal('exercise-search-modal');
    renderPlanExercises();
}

// ============================================================================
// Modals
// ============================================================================

function openSetModal(exerciseId, exerciseName, setNumber) {
    const sets = state.todaySets[exerciseId] || [];
    const existingSet = sets.find(s => s.set_number === setNumber);
    
    // Find previous set's weight for default
    let defaultWeight = '';
    if (!existingSet?.weight) {
        const prevSet = sets.filter(s => s.set_number < setNumber && s.weight).pop();
        if (prevSet) defaultWeight = prevSet.weight;
    }
    
    state.editingSet = { exerciseId, setNumber };
    
    document.getElementById('set-exercise-name').textContent = exerciseName;
    document.getElementById('set-number').textContent = `Set ${setNumber}`;
    document.getElementById('set-weight').value = existingSet?.weight || defaultWeight || '';
    document.getElementById('set-reps').value = existingSet?.reps || '';
    
    document.getElementById('set-modal').classList.remove('hidden');
    document.getElementById('set-weight').focus();
}

function openExerciseModal(exercise = null) {
    state.editingExercise = exercise;
    document.getElementById('exercise-modal-title').textContent = exercise ? 'Edit Exercise' : 'New Exercise';
    document.getElementById('exercise-name').value = exercise?.name || '';
    document.getElementById('exercise-modal').classList.remove('hidden');
    document.getElementById('exercise-name').focus();
}

function closeModal(modalId) {
    document.getElementById(modalId).classList.add('hidden');
}

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

// ============================================================================
// Event Handlers
// ============================================================================

function initEventHandlers() {
    // Auth tabs
    document.querySelectorAll('.auth-tabs .tab').forEach(tab => {
        tab.addEventListener('click', () => {
            const isRegister = tab.dataset.tab === 'register';
            document.querySelectorAll('.auth-tabs .tab').forEach(t => t.classList.remove('active'));
            tab.classList.add('active');
            document.getElementById('qr-section').classList.toggle('hidden', !isRegister);
            document.getElementById('auth-submit').textContent = isRegister ? 'Register' : 'Login';
            
            if (isRegister) {
                state.totpSecret = generateTotpSecret();
                document.getElementById('totp-secret').textContent = state.totpSecret;
                generateQRCode(state.totpSecret, document.getElementById('username').value);
            }
        });
    });
    
    // Update QR when username changes
    document.getElementById('username').addEventListener('input', (e) => {
        if (!document.getElementById('qr-section').classList.contains('hidden')) {
            generateQRCode(state.totpSecret, e.target.value);
        }
    });
    
    // Auth form
    document.getElementById('auth-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        const isRegister = document.querySelector('.auth-tabs .tab.active').dataset.tab === 'register';
        const username = document.getElementById('username').value;
        const password = document.getElementById('password').value;
        const totp = document.getElementById('totp').value;
        const status = document.getElementById('auth-status');
        
        try {
            document.getElementById('auth-submit').disabled = true;
            if (isRegister) {
                await register(username, password, state.totpSecret, totp);
            } else {
                await login(username, password, totp);
            }
        } catch (e) {
            status.textContent = e.message;
            status.className = 'status error';
        } finally {
            document.getElementById('auth-submit').disabled = false;
        }
    });
    
    // Navigation
    document.querySelectorAll('.nav-btn').forEach(btn => {
        btn.addEventListener('click', () => switchTab(btn.dataset.view));
    });
    
    // Day navigation
    document.getElementById('prev-day').addEventListener('click', () => {
        state.currentDate.setDate(state.currentDate.getDate() - 1);
        updateDayDisplay();
        loadDaySets();
    });
    
    document.getElementById('next-day').addEventListener('click', () => {
        state.currentDate.setDate(state.currentDate.getDate() + 1);
        updateDayDisplay();
        loadDaySets();
    });
    
    document.getElementById('today-btn').addEventListener('click', () => {
        state.currentDate = new Date();
        updateDayDisplay();
        loadDaySets();
    });
    
    // Plans
    document.getElementById('back-to-plans').addEventListener('click', () => {
        document.getElementById('weekday-list').classList.remove('hidden');
        document.getElementById('plan-detail').classList.add('hidden');
    });
    
    document.getElementById('add-to-plan').addEventListener('click', () => {
        document.getElementById('exercise-search').value = '';
        renderSearchResults('');
        document.getElementById('exercise-search-modal').classList.remove('hidden');
        document.getElementById('exercise-search').focus();
    });
    
    document.getElementById('close-search').addEventListener('click', () => {
        closeModal('exercise-search-modal');
    });
    
    document.getElementById('exercise-search').addEventListener('input', (e) => {
        renderSearchResults(e.target.value);
    });
    
    // Exercises
    document.getElementById('show-archived').addEventListener('change', renderAllExercises);
    
    document.getElementById('add-exercise-btn').addEventListener('click', () => {
        openExerciseModal();
    });
    
    document.getElementById('close-exercise-modal').addEventListener('click', () => {
        closeModal('exercise-modal');
    });
    
    document.getElementById('cancel-exercise').addEventListener('click', () => {
        closeModal('exercise-modal');
    });
    
    document.getElementById('save-exercise').addEventListener('click', async () => {
        const name = document.getElementById('exercise-name').value.trim();
        if (!name) return;
        
        try {
            if (state.editingExercise) {
                await updateExercise(state.editingExercise.id, { name });
            } else {
                await createExercise(name);
            }
            closeModal('exercise-modal');
        } catch (e) {
            showStatus(e.message, 'error');
        }
    });
    
    // Set modal
    document.getElementById('close-set-modal').addEventListener('click', () => {
        closeModal('set-modal');
    });
    
    document.querySelectorAll('.weight-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            const input = document.getElementById('set-weight');
            const delta = parseFloat(btn.dataset.delta);
            const current = parseFloat(input.value) || 0;
            input.value = Math.max(0, current + delta);
        });
    });
    
    document.getElementById('save-set').addEventListener('click', async () => {
        if (!state.editingSet) return;
        const weight = parseFloat(document.getElementById('set-weight').value) || null;
        const reps = parseInt(document.getElementById('set-reps').value) || null;
        
        try {
            await saveSet(state.editingSet.exerciseId, state.editingSet.setNumber, weight, reps);
            closeModal('set-modal');
        } catch (e) {
            showStatus(e.message, 'error');
        }
    });
    
    document.getElementById('delete-set').addEventListener('click', async () => {
        if (!state.editingSet) return;
        
        try {
            await deleteSet(state.editingSet.exerciseId, state.editingSet.setNumber);
            closeModal('set-modal');
        } catch (e) {
            showStatus(e.message, 'error');
        }
    });
    
    // Close modals on backdrop click
    document.querySelectorAll('.modal').forEach(modal => {
        modal.addEventListener('click', (e) => {
            if (e.target === modal) {
                modal.classList.add('hidden');
            }
        });
    });
    
    // Keyboard shortcuts
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            document.querySelectorAll('.modal:not(.hidden)').forEach(modal => {
                modal.classList.add('hidden');
            });
        }
    });
}

// ============================================================================
// Init
// ============================================================================

document.addEventListener('DOMContentLoaded', () => {
    initEventHandlers();
    checkSession();
});
