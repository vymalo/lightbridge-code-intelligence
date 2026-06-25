"""Feedback-quality dashboard — how the team reacts to the bot's review comments.

GitHub reactions on our posted comments are reconciled into `review_feedback` (ADR-0035) by the poller.
Joining a reaction back to the finding it was on goes via `review_comments` (the inline comment's
file/line) → the matching object in `reviews.findings` (priority/category). So a 👎 can be attributed
to a finding's priority/category — the signal for "which kinds of findings land badly".
"""

from __future__ import annotations

from grafana_foundation_sdk.builders import bargauge, dashboard, stat, table, timeseries

from .common import POSTGRES, Layout, sql

UID = "lci-feedback"

# A reacted INLINE comment resolved back to its finding: review_feedback → review_comments (by GitHub
# comment id + kind) → reviews.findings (by file/line). Line compared as text to avoid a cast on any
# non-numeric value. `rf.created_at` is the reaction's reconcile time (the feedback timeline).
_REACTED_FINDING = (
    "FROM review_feedback rf "
    "JOIN review_comments rc ON rc.github_comment_id = rf.github_comment_id AND rc.kind = rf.comment_kind "
    "JOIN reviews rv ON rv.task_id = rc.task_id "
    "JOIN tasks t ON t.id = rc.task_id "
    "LEFT JOIN repositories r ON r.id = t.repository_id "
    "CROSS JOIN LATERAL jsonb_array_elements(rv.findings) f "
    "WHERE rc.kind = 'inline' AND f->>'file' = rc.file AND f->>'line' = rc.line::text"
)


def dashboard_builder() -> dashboard.Dashboard:
    layout = Layout()

    total = (
        stat.Panel()
        .title("Reactions")
        .datasource(POSTGRES)
        .with_target(sql("SELECT count(*) FROM review_feedback WHERE $__timeFilter(created_at)"))
        .grid_pos(layout.place(6, 4))
    )
    approval = (
        stat.Panel()
        .title("Approval rate")
        .datasource(POSTGRES)
        .unit("percentunit")
        .with_target(
            sql(
                "SELECT count(*) FILTER (WHERE reaction = '+1')::float "
                "/ NULLIF(count(*) FILTER (WHERE reaction IN ('+1','-1')), 0) "
                "FROM review_feedback WHERE $__timeFilter(created_at)"
            )
        )
        .grid_pos(layout.place(6, 4))
    )
    downvotes = (
        stat.Panel()
        .title("👎 (range)")
        .datasource(POSTGRES)
        .with_target(
            sql("SELECT count(*) FROM review_feedback WHERE reaction = '-1' AND $__timeFilter(created_at)")
        )
        .grid_pos(layout.place(6, 4))
    )
    reactors = (
        stat.Panel()
        .title("Distinct reactors")
        .datasource(POSTGRES)
        .with_target(
            sql("SELECT count(DISTINCT reactor) FROM review_feedback WHERE $__timeFilter(created_at)")
        )
        .grid_pos(layout.place(6, 4))
    )

    over_time = (
        timeseries.Panel()
        .title("Reactions over time")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT $__timeGroupAlias(created_at, $__interval), reaction AS \"reaction\", "
                "count(*) AS \"count\" "
                "FROM review_feedback WHERE $__timeFilter(created_at) GROUP BY 1, 2 ORDER BY 1",
                fmt="time_series",
            )
        )
        .grid_pos(layout.place(12, 8))
    )
    by_reaction = (
        bargauge.Panel()
        .title("Reactions by type")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT reaction AS metric, count(*) AS value "
                "FROM review_feedback WHERE $__timeFilter(created_at) GROUP BY reaction ORDER BY value DESC"
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    # Which finding categories/priorities draw 👎 — the "what lands badly" signal.
    downvote_category = (
        bargauge.Panel()
        .title("👎 by finding category")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT coalesce(nullif(f->>'category',''),'correctness') AS metric, count(*) AS value "
                f"{_REACTED_FINDING} AND rf.reaction = '-1' AND $__timeFilter(rf.created_at) "
                "GROUP BY 1 ORDER BY value DESC"
            )
        )
        .grid_pos(layout.place(12, 8))
    )
    downvote_priority = (
        bargauge.Panel()
        .title("👎 by finding priority")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT coalesce(nullif(f->>'priority',''),'P2') AS metric, count(*) AS value "
                f"{_REACTED_FINDING} AND rf.reaction = '-1' AND $__timeFilter(rf.created_at) "
                "GROUP BY 1 ORDER BY value DESC"
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    per_repo = (
        table.Panel()
        .title("Feedback by repository")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT coalesce(r.owner || '/' || r.name, t.repository_id::text) AS \"repository\", "
                "count(*) FILTER (WHERE rf.reaction = '+1') AS \"up\", "
                "count(*) FILTER (WHERE rf.reaction = '-1') AS \"down\", "
                "count(*) FILTER (WHERE rf.reaction NOT IN ('+1','-1')) AS \"other\" "
                "FROM review_feedback rf JOIN tasks t ON t.id = rf.task_id "
                "LEFT JOIN repositories r ON r.id = t.repository_id "
                "WHERE $__timeFilter(rf.created_at) "
                "GROUP BY r.owner, r.name, t.repository_id "
                "ORDER BY count(*) FILTER (WHERE rf.reaction = '-1') DESC, \"up\" DESC"
            )
        )
        .grid_pos(layout.place(12, 9))
    )
    top_downvoted = (
        table.Panel()
        .title("Most down-voted findings")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT coalesce(r.owner || '/' || r.name, t.repository_id::text) AS \"repository\", "
                "rc.file AS \"file\", rc.line AS \"line\", "
                "coalesce(nullif(f->>'priority',''),'P2') AS \"priority\", "
                "coalesce(nullif(f->>'category',''),'correctness') AS \"category\", "
                "f->>'title' AS \"finding\", count(*) AS \"downvotes\" "
                f"{_REACTED_FINDING} AND rf.reaction = '-1' AND $__timeFilter(rf.created_at) "
                "GROUP BY r.owner, r.name, t.repository_id, rc.file, rc.line, "
                "f->>'priority', f->>'category', f->>'title' "
                "ORDER BY count(*) DESC LIMIT 50"
            )
        )
        .grid_pos(layout.place(12, 9))
    )

    return (
        dashboard.Dashboard("Lightbridge — Feedback quality")
        .uid(UID)
        .tags(["lightbridge", "generated"])
        .refresh("1m")
        .time("now-30d", "now")
        .with_panel(total)
        .with_panel(approval)
        .with_panel(downvotes)
        .with_panel(reactors)
        .with_panel(over_time)
        .with_panel(by_reaction)
        .with_panel(downvote_category)
        .with_panel(downvote_priority)
        .with_panel(per_repo)
        .with_panel(top_downvoted)
    )
