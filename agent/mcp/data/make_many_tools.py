"""Generates many-tools.json (a catalogue shaped like real MCP servers) and many-tools-queries.json (requests worded
unlike the descriptions, with the tool that does each) for ADR-139's tests and measurement. Run from the repository root."""
import json

R = True  # read-only
W = False

T = []
def t(server, name, desc, args, ro, req=None):
    props = {k: ({"type": "string", "description": v} if isinstance(v, str) else v) for k, v in args.items()}
    T.append({
        "name": f"{server}_{name}",
        "description": desc,
        "inputSchema": {"type": "object", "properties": props, "required": req if req is not None else list(args)[:1]},
        "annotations": {"readOnlyHint": ro},
    })

N = {"type": "integer", "description": "Page size (max 100)"}
# GitHub
g = "github"
t(g, "create_issue", "Create a new issue in a GitHub repository.", {"repo": "owner/name", "title": "Issue title", "body": "Markdown body", "labels": {"type": "array", "items": {"type": "string"}, "description": "Label names"}}, W, ["repo", "title"])
t(g, "update_issue", "Edit an existing issue: title, body, state (open/closed), assignees or labels.", {"repo": "owner/name", "number": {"type": "integer"}, "state": {"type": "string", "enum": ["open", "closed"]}}, W, ["repo", "number"])
t(g, "list_issues", "List issues in a repository, filtered by state, labels, assignee or creation date.", {"repo": "owner/name", "state": "open, closed or all", "labels": "Comma-separated labels", "per_page": N}, R)
t(g, "get_issue", "Get one issue with its body, labels, assignees and state.", {"repo": "owner/name", "number": {"type": "integer"}}, R, ["repo", "number"])
t(g, "add_issue_comment", "Add a comment to an issue or pull request.", {"repo": "owner/name", "number": {"type": "integer"}, "body": "Comment text"}, W, ["repo", "number", "body"])
t(g, "search_issues", "Search issues and pull requests across GitHub with the search syntax (is:open label:bug).", {"q": "Search query"}, R)
t(g, "create_pull_request", "Open a pull request from a head branch into a base branch.", {"repo": "owner/name", "head": "Branch with the changes", "base": "Branch to merge into", "title": "PR title"}, W, ["repo", "head", "base", "title"])
t(g, "list_pull_requests", "List pull requests of a repository by state and base branch.", {"repo": "owner/name", "state": "open, closed or all"}, R)
t(g, "get_pull_request_diff", "Get the unified diff of a pull request.", {"repo": "owner/name", "number": {"type": "integer"}}, R, ["repo", "number"])
t(g, "merge_pull_request", "Merge a pull request with merge, squash or rebase.", {"repo": "owner/name", "number": {"type": "integer"}, "method": {"type": "string", "enum": ["merge", "squash", "rebase"]}}, W, ["repo", "number"])
t(g, "request_reviewers", "Request reviews on a pull request from users or teams.", {"repo": "owner/name", "number": {"type": "integer"}, "reviewers": {"type": "array", "items": {"type": "string"}}}, W, ["repo", "number", "reviewers"])
t(g, "get_file_contents", "Read a file or list a directory of a repository at a branch, tag or commit.", {"repo": "owner/name", "path": "File or directory path", "ref": "Branch, tag or SHA"}, R, ["repo", "path"])
t(g, "create_or_update_file", "Create or replace a file in a repository with a commit.", {"repo": "owner/name", "path": "File path", "content": "New content", "message": "Commit message", "branch": "Branch"}, W, ["repo", "path", "content", "message"])
t(g, "create_branch", "Create a branch from another branch or commit.", {"repo": "owner/name", "branch": "New branch name", "from": "Source branch or SHA"}, W, ["repo", "branch"])
t(g, "list_commits", "List the commits of a branch, newest first.", {"repo": "owner/name", "sha": "Branch or SHA", "per_page": N}, R)
t(g, "search_code", "Search source code across repositories (language:rust repo:owner/name).", {"q": "Code search query"}, R)
t(g, "list_workflow_runs", "List GitHub Actions workflow runs with their status and conclusion.", {"repo": "owner/name", "branch": "Branch", "status": "queued, in_progress, completed"}, R)
t(g, "rerun_workflow", "Re-run a failed GitHub Actions workflow run.", {"repo": "owner/name", "run_id": {"type": "integer"}}, W, ["repo", "run_id"])
t(g, "create_release", "Publish a release for a tag with release notes.", {"repo": "owner/name", "tag": "Tag name", "name": "Release title", "body": "Release notes"}, W, ["repo", "tag"])
t(g, "list_releases", "List the releases of a repository.", {"repo": "owner/name"}, R)
# Slack
s = "slack"
t(s, "post_message", "Post a message to a channel or a thread.", {"channel": "Channel id or #name", "text": "Message (mrkdwn)", "thread_ts": "Reply in this thread"}, W, ["channel", "text"])
t(s, "list_channels", "List public and private channels the bot is in.", {"limit": N}, R, [])
t(s, "get_channel_history", "Read the latest messages of a channel.", {"channel": "Channel id", "limit": N}, R)
t(s, "get_thread_replies", "Read the replies of a message thread.", {"channel": "Channel id", "ts": "Parent message timestamp"}, R, ["channel", "ts"])
t(s, "search_messages", "Search messages in the workspace (from:@user in:#channel).", {"query": "Search query"}, R)
t(s, "add_reaction", "Add an emoji reaction to a message.", {"channel": "Channel id", "ts": "Message timestamp", "emoji": "Emoji name"}, W, ["channel", "ts", "emoji"])
t(s, "send_direct_message", "Send a direct message to a user.", {"user": "User id or email", "text": "Message"}, W, ["user", "text"])
t(s, "get_user_profile", "Get a user's profile: name, title, timezone, status.", {"user": "User id"}, R)
t(s, "set_channel_topic", "Set the topic of a channel.", {"channel": "Channel id", "topic": "New topic"}, W, ["channel", "topic"])
t(s, "upload_file", "Upload a file or snippet to a channel.", {"channel": "Channel id", "filename": "Name", "content": "File content"}, W, ["channel", "content"])
t(s, "schedule_message", "Schedule a message to be posted later.", {"channel": "Channel id", "text": "Message", "post_at": "Unix time"}, W, ["channel", "text", "post_at"])
# Jira
j = "jira"
t(j, "create_issue", "Create a Jira issue (story, task, bug) in a project.", {"project": "Project key", "summary": "Summary", "type": {"type": "string", "enum": ["Story", "Task", "Bug", "Epic"]}, "description": "Description"}, W, ["project", "summary"])
t(j, "transition_issue", "Move a Jira issue through its workflow (To Do, In Progress, Done).", {"key": "Issue key", "transition": "Transition name"}, W, ["key", "transition"])
t(j, "search_jql", "Search Jira issues with JQL.", {"jql": "JQL query", "max_results": N}, R)
t(j, "get_issue", "Get a Jira issue with its fields, comments and links.", {"key": "Issue key"}, R)
t(j, "add_comment", "Comment on a Jira issue.", {"key": "Issue key", "body": "Comment"}, W, ["key", "body"])
t(j, "assign_issue", "Assign a Jira issue to a user.", {"key": "Issue key", "account_id": "Assignee"}, W, ["key", "account_id"])
t(j, "log_work", "Log time spent on a Jira issue.", {"key": "Issue key", "time_spent": "e.g. 2h 30m", "comment": "What was done"}, W, ["key", "time_spent"])
t(j, "list_sprints", "List the sprints of a board (active, future, closed).", {"board_id": {"type": "integer"}, "state": "active, future, closed"}, R)
t(j, "move_to_sprint", "Move issues into a sprint.", {"sprint_id": {"type": "integer"}, "issues": {"type": "array", "items": {"type": "string"}}}, W, ["sprint_id", "issues"])
t(j, "link_issues", "Link two Jira issues (blocks, relates to, duplicates).", {"inward": "Issue key", "outward": "Issue key", "type": "Link type"}, W, ["inward", "outward", "type"])
# Sentry
e = "sentry"
t(e, "list_issues", "List the unresolved error issues of a project, most frequent first.", {"project": "Project slug", "query": "Search (is:unresolved level:error)", "stats_period": "24h, 14d"}, R)
t(e, "get_issue_details", "Get an error issue with its latest event, stack trace and tags.", {"issue_id": "Issue id"}, R)
t(e, "get_event", "Get one event with its full stack trace, breadcrumbs and request.", {"event_id": "Event id", "project": "Project slug"}, R, ["event_id"])
t(e, "resolve_issue", "Mark an error issue resolved, optionally in the next release.", {"issue_id": "Issue id", "in_next_release": {"type": "boolean"}}, W, ["issue_id"])
t(e, "list_releases", "List releases of a project with their new issue counts.", {"project": "Project slug"}, R)
t(e, "get_performance_summary", "Get p50/p95 latency and throughput of transactions.", {"project": "Project slug", "transaction": "Transaction name"}, R)
# Linear
l = "linear"
t(l, "create_issue", "Create a Linear issue in a team with priority and labels.", {"team": "Team key", "title": "Title", "priority": {"type": "integer", "description": "0 none .. 4 low; 1 urgent"}}, W, ["team", "title"])
t(l, "update_issue", "Update a Linear issue: state, assignee, priority, estimate.", {"id": "Issue id", "state": "Workflow state"}, W, ["id"])
t(l, "list_my_issues", "List the issues assigned to me, by state.", {"state": "Workflow state"}, R, [])
t(l, "list_cycles", "List a team's cycles with their progress.", {"team": "Team key"}, R)
t(l, "list_projects", "List Linear projects with lead, status and target date.", {}, R, [])
# Notion
n = "notion"
t(n, "search", "Search pages and databases by title in the Notion workspace.", {"query": "Text"}, R)
t(n, "get_page", "Read a Notion page's properties and content blocks.", {"page_id": "Page id"}, R)
t(n, "create_page", "Create a page under a page or in a database.", {"parent": "Parent page or database id", "title": "Title", "content": "Markdown content"}, W, ["parent", "title"])
t(n, "append_blocks", "Append paragraphs, lists or headings to a page.", {"page_id": "Page id", "content": "Markdown"}, W, ["page_id", "content"])
t(n, "query_database", "Query a Notion database with filters and sorts.", {"database_id": "Database id", "filter": {"type": "object"}}, R, ["database_id"])
t(n, "update_page_properties", "Update the properties of a database row.", {"page_id": "Page id", "properties": {"type": "object"}}, W, ["page_id", "properties"])
# Postgres
p = "postgres"
t(p, "query", "Run a read-only SQL query and return the rows.", {"sql": "SELECT statement"}, R)
t(p, "execute", "Run a SQL statement that changes data or schema (INSERT, UPDATE, CREATE).", {"sql": "Statement"}, W)
t(p, "list_tables", "List the tables of a schema with their row estimates.", {"schema": "Schema name"}, R, [])
t(p, "describe_table", "Show a table's columns, types, indexes and constraints.", {"table": "Table name"}, R)
t(p, "explain_query", "Show the query plan of a SQL statement (EXPLAIN ANALYZE).", {"sql": "Statement"}, R)
t(p, "list_slow_queries", "List the slowest queries from pg_stat_statements.", {"limit": N}, R, [])
# Kubernetes
k = "k8s"
t(k, "get_pods", "List pods of a namespace with status, restarts and age.", {"namespace": "Namespace", "selector": "Label selector"}, R, [])
t(k, "get_pod_logs", "Read the logs of a pod's container.", {"pod": "Pod name", "namespace": "Namespace", "tail": {"type": "integer"}, "previous": {"type": "boolean", "description": "Logs of the previous (crashed) container"}}, R)
t(k, "describe_resource", "Describe a resource (pod, deployment, service, node) with its events.", {"kind": "Resource kind", "name": "Name", "namespace": "Namespace"}, R, ["kind", "name"])
t(k, "scale_deployment", "Scale a deployment to a number of replicas.", {"deployment": "Name", "namespace": "Namespace", "replicas": {"type": "integer"}}, W, ["deployment", "replicas"])
t(k, "rollout_restart", "Restart a deployment's pods with a rolling restart.", {"deployment": "Name", "namespace": "Namespace"}, W)
t(k, "rollout_undo", "Roll a deployment back to its previous revision.", {"deployment": "Name", "namespace": "Namespace"}, W)
t(k, "apply_manifest", "Apply a YAML manifest to the cluster.", {"manifest": "YAML"}, W)
t(k, "get_events", "List recent cluster events (warnings first).", {"namespace": "Namespace"}, R, [])
t(k, "top_pods", "Show CPU and memory use of pods.", {"namespace": "Namespace"}, R, [])
# Grafana / Datadog
gf = "grafana"
t(gf, "query_prometheus", "Run a PromQL query over a time range.", {"expr": "PromQL", "start": "Start time", "end": "End time"}, R, ["expr"])
t(gf, "query_loki", "Search logs with LogQL over a time range.", {"expr": "LogQL", "since": "e.g. 1h"}, R, ["expr"])
t(gf, "list_dashboards", "Search dashboards by title or tag.", {"query": "Text"}, R, [])
t(gf, "get_dashboard_panel_data", "Get the data behind a dashboard panel.", {"dashboard_uid": "Dashboard", "panel_id": {"type": "integer"}}, R, ["dashboard_uid", "panel_id"])
t(gf, "list_alert_rules", "List alert rules with their state (firing, pending, normal).", {"state": "firing, pending, normal"}, R, [])
t(gf, "silence_alert", "Silence an alert for a duration.", {"matchers": "Label matchers", "duration": "e.g. 2h", "comment": "Why"}, W, ["matchers", "duration"])
t(gf, "create_annotation", "Add an annotation (deploy, incident) to dashboards at a time.", {"text": "Annotation text", "tags": {"type": "array", "items": {"type": "string"}}}, W)
# PagerDuty
pd = "pagerduty"
t(pd, "list_incidents", "List triggered and acknowledged incidents.", {"status": "triggered, acknowledged, resolved"}, R, [])
t(pd, "acknowledge_incident", "Acknowledge an incident so escalation stops.", {"incident_id": "Incident id"}, W)
t(pd, "resolve_incident", "Resolve an incident.", {"incident_id": "Incident id", "note": "Resolution note"}, W, ["incident_id"])
t(pd, "get_oncall", "Who is on call now for a schedule or escalation policy.", {"schedule": "Schedule id or name"}, R, [])
t(pd, "create_incident", "Page a service by creating an incident.", {"service": "Service id", "title": "What is wrong", "urgency": {"type": "string", "enum": ["high", "low"]}}, W, ["service", "title"])
# AWS S3 / CloudWatch
a = "aws"
t(a, "s3_list_buckets", "List the S3 buckets of the account.", {}, R, [])
t(a, "s3_list_objects", "List objects of a bucket under a prefix.", {"bucket": "Bucket", "prefix": "Key prefix"}, R)
t(a, "s3_get_object", "Read an object's content from a bucket.", {"bucket": "Bucket", "key": "Object key"}, R, ["bucket", "key"])
t(a, "s3_put_object", "Upload content as an object to a bucket.", {"bucket": "Bucket", "key": "Object key", "content": "Body"}, W, ["bucket", "key", "content"])
t(a, "s3_presign_url", "Create a time-limited download link for an object.", {"bucket": "Bucket", "key": "Object key", "expires": {"type": "integer", "description": "Seconds"}}, R, ["bucket", "key"])
t(a, "cloudwatch_get_metric", "Get a CloudWatch metric's datapoints (CPUUtilization, Latency).", {"namespace": "AWS/EC2", "metric": "Metric name", "dimensions": {"type": "object"}}, R, ["namespace", "metric"])
t(a, "cloudwatch_logs_insights", "Run a CloudWatch Logs Insights query over log groups.", {"log_groups": {"type": "array", "items": {"type": "string"}}, "query": "Insights query"}, R, ["log_groups", "query"])
t(a, "lambda_invoke", "Invoke a Lambda function with a JSON payload.", {"function": "Function name", "payload": {"type": "object"}}, W)
t(a, "ec2_describe_instances", "List EC2 instances with state, type and IPs.", {"filters": {"type": "object"}}, R, [])
# Stripe
st = "stripe"
t(st, "list_customers", "List or search customers by email.", {"email": "Email"}, R, [])
t(st, "get_customer", "Get a customer with their subscriptions and payment methods.", {"customer": "Customer id"}, R)
t(st, "create_refund", "Refund a charge or payment intent, fully or partly.", {"payment_intent": "Payment intent id", "amount": {"type": "integer", "description": "Cents; empty for all"}}, W, ["payment_intent"])
t(st, "list_invoices", "List invoices of a customer with their status.", {"customer": "Customer id", "status": "draft, open, paid, void"}, R, [])
t(st, "cancel_subscription", "Cancel a subscription now or at period end.", {"subscription": "Subscription id", "at_period_end": {"type": "boolean"}}, W, ["subscription"])
t(st, "create_payment_link", "Create a payment link for a price.", {"price": "Price id", "quantity": {"type": "integer"}}, W, ["price"])
t(st, "get_balance", "Get the account's available and pending balance.", {}, R, [])
# Google Calendar / Gmail
c = "google"
t(c, "calendar_list_events", "List calendar events in a time range.", {"time_min": "Start", "time_max": "End", "calendar": "Calendar id"}, R, [])
t(c, "calendar_create_event", "Create a calendar event and invite attendees.", {"title": "Title", "start": "Start", "end": "End", "attendees": {"type": "array", "items": {"type": "string"}}}, W, ["title", "start", "end"])
t(c, "calendar_find_free_time", "Find times when all given people are free.", {"people": {"type": "array", "items": {"type": "string"}}, "duration_minutes": {"type": "integer"}}, R, ["people"])
t(c, "gmail_search", "Search emails with Gmail query syntax (from: subject: newer_than:).", {"q": "Query"}, R)
t(c, "gmail_read", "Read an email thread.", {"thread_id": "Thread id"}, R)
t(c, "gmail_send", "Send an email.", {"to": "Recipients", "subject": "Subject", "body": "Body"}, W, ["to", "subject", "body"])
t(c, "gmail_draft", "Create an email draft without sending it.", {"to": "Recipients", "subject": "Subject", "body": "Body"}, W, ["to", "subject", "body"])
t(c, "drive_search", "Search Google Drive files by name or content.", {"q": "Text"}, R)
t(c, "drive_read", "Read a Google Doc, Sheet or file's content.", {"file_id": "File id"}, R)
# Docker / registry
d = "docker"
t(d, "list_containers", "List running containers with image, status and ports.", {"all": {"type": "boolean"}}, R, [])
t(d, "container_logs", "Read a container's logs.", {"container": "Name or id", "tail": {"type": "integer"}}, R)
t(d, "restart_container", "Restart a container.", {"container": "Name or id"}, W)
t(d, "list_images", "List local images with tags and size.", {}, R, [])
t(d, "image_vulnerabilities", "Scan an image for known CVEs.", {"image": "Image reference"}, R)
# Confluence
cf = "confluence"
t(cf, "search", "Search Confluence pages with CQL or text.", {"query": "Text or CQL"}, R)
t(cf, "get_page", "Read a Confluence page's body.", {"page_id": "Page id"}, R)
t(cf, "create_page", "Create a Confluence page in a space.", {"space": "Space key", "title": "Title", "body": "Storage-format or markdown body"}, W, ["space", "title", "body"])
t(cf, "update_page", "Replace a Confluence page's body (new version).", {"page_id": "Page id", "body": "New body"}, W, ["page_id", "body"])
# Feature flags / misc
f = "flags"
t(f, "list_flags", "List feature flags with their state per environment.", {"environment": "production, staging"}, R, [])
t(f, "toggle_flag", "Turn a feature flag on or off in an environment.", {"flag": "Flag key", "environment": "Environment", "on": {"type": "boolean"}}, W, ["flag", "environment", "on"])
t(f, "set_rollout", "Roll a flag out to a percentage of users.", {"flag": "Flag key", "percent": {"type": "integer"}}, W, ["flag", "percent"])
# the one only a search finds (eval target)
t("releases", "codename", "Look up the codename the release team gave a product version (e.g. 4.2 -> its codename).", {"version": "Product version, e.g. 4.2"}, R)

open("agent/mcp/data/many-tools.json", "w", encoding="utf-8", newline="\n").write(json.dumps(T, indent=1) + "\n")
print(len(T), "tools")

Q = [
 ("open a bug report on our GitHub repo", "github_create_issue"),
 ("close github issue 42", "github_update_issue"),
 ("which pull requests are waiting to be merged", "github_list_pull_requests"),
 ("see what changed in PR 17", "github_get_pull_request_diff"),
 ("squash and merge the pull request", "github_merge_pull_request"),
 ("ask alice to review my PR", "github_request_reviewers"),
 ("read README.md from the main branch of the repo", "github_get_file_contents"),
 ("why did the CI build fail on main", "github_list_workflow_runs"),
 ("retry the failed CI job", "github_rerun_workflow"),
 ("publish release notes for tag v2.0", "github_create_release"),
 ("tell the team in #deploys that the release is out", "slack_post_message"),
 ("what did people say in the incident channel today", "slack_get_channel_history"),
 ("DM bob that the meeting moved", "slack_send_direct_message"),
 ("find the slack conversation about the outage", "slack_search_messages"),
 ("put a thumbs up on that message", "slack_add_reaction"),
 ("move PROJ-12 to done", "jira_transition_issue"),
 ("record 3 hours spent on PROJ-7", "jira_log_work"),
 ("find all open bugs in project WEB with JQL", "jira_search_jql"),
 ("PROJ-9 blocks PROJ-10", "jira_link_issues"),
 ("what exceptions is production throwing", "sentry_list_issues"),
 ("show me the stack trace of that crash", "sentry_get_event"),
 ("the error is fixed, mark it done in sentry", "sentry_resolve_issue"),
 ("how slow is the checkout endpoint p95", "sentry_get_performance_summary"),
 ("what Linear tickets are on my plate", "linear_list_my_issues"),
 ("write meeting notes as a new Notion page", "notion_create_page"),
 ("filter the roadmap database for items due this month", "notion_query_database"),
 ("how many users signed up yesterday (SQL)", "postgres_query"),
 ("what columns does the orders table have", "postgres_describe_table"),
 ("why is this query slow, show the plan", "postgres_explain_query"),
 ("add an index on orders.created_at", "postgres_execute"),
 ("is the api pod crash looping", "k8s_get_pods"),
 ("logs of the container that crashed before the restart", "k8s_get_pod_logs"),
 ("bump the web deployment to 5 replicas", "k8s_scale_deployment"),
 ("the new version is broken, go back to the previous one", "k8s_rollout_undo"),
 ("which pods use the most memory", "k8s_top_pods"),
 ("graph the request rate from prometheus for the last hour", "grafana_query_prometheus"),
 ("search the application logs for timeout errors", "grafana_query_loki"),
 ("which alerts are firing right now", "grafana_list_alert_rules"),
 ("mute the disk alert for two hours", "grafana_silence_alert"),
 ("mark the deploy on the dashboards", "grafana_create_annotation"),
 ("who is on call tonight", "pagerduty_get_oncall"),
 ("stop paging, I am looking at it", "pagerduty_acknowledge_incident"),
 ("wake up the database team, the db is down", "pagerduty_create_incident"),
 ("give the customer a download link for the report in s3", "aws_s3_presign_url"),
 ("CPU usage of the EC2 fleet", "aws_cloudwatch_get_metric"),
 ("give the customer their money back", "stripe_create_refund"),
 ("stop the customer's plan at the end of the month", "stripe_cancel_subscription"),
 ("how much money is in the stripe account", "stripe_get_balance"),
 ("when are bob and carol both available for 30 minutes", "google_calendar_find_free_time"),
 ("set up a meeting with the design team on friday", "google_calendar_create_event"),
 ("email the client the invoice", "google_gmail_send"),
 ("prepare an email but don't send it yet", "google_gmail_draft"),
 ("is the image affected by known security holes", "docker_image_vulnerabilities"),
 ("restart the redis container", "docker_restart_container"),
 ("update the runbook page in confluence", "confluence_update_page"),
 ("turn off the new checkout feature in production", "flags_toggle_flag"),
 ("enable the beta for 10% of users", "flags_set_rollout"),
 ("what is the codename of version 4.2", "releases_codename"),
]
json.dump([{"query": q, "expect": e} for q, e in Q], open("agent/mcp/data/many-tools-queries.json", "w", encoding="utf-8", newline="\n"), indent=1)
names = {x["name"] for x in T}
assert all(e in names for _, e in Q), [e for _, e in Q if e not in names]
print(len(Q), "queries")
