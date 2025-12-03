# Ekman

A TUI client/server gym progress tracker app.

## API overview (current prototype)

All endpoints are scoped to the authenticated user (session cookie). Exercises can be user-owned or global (admin); mutations are only allowed on user-owned exercises.

- Auth: `POST /api/auth/register`, `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me`, `GET /api/auth/totp/setup`, `POST /api/auth/totp/enable`
- Plans: `GET /api/plans/daily`
- Activity: `GET /api/activity/days?start=&end=`
- Exercises:
  - `GET /api/exercises` (returns both global and user exercises, with `owner`)
  - `GET /api/exercises/{id}`
  - `POST /api/exercises`
  - `PATCH /api/exercises/{id}` (updates name/description/archived; returns the updated exercise)
  - `POST /api/exercises/{id}/archive` (archives; returns the archived exercise)
  - `GET /api/exercises/{id}/graph?start=&end=&metric=`
- Sets:
  - `GET /api/days/{date}/exercises/{exercise_id}/sets` (list ordered sets for that day/exercise)
  - `PUT /api/days/{date}/exercises/{exercise_id}/sets/{set_number}` (create/update; `completed_at` time is clamped to the path date)
  - `DELETE /api/days/{date}/exercises/{exercise_id}/sets/{set_number}`

All dates in paths use `YYYY-MM-DD`; timestamps are UTC. Set uniqueness is `(exercise_id, day, set_number)`.
