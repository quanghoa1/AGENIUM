## find_code A/B: grok grok-4.7 on flask (usual read tools vs + find_code)

| # | right (without / with) | tokens in (without / with) | out (without / with) | steps (without / with) | Jev tokens | tools used with find_code |
|---|---|---|---|---|---|---|
| 1 | yes / yes | 153164 / 94359 | 1451 / 1087 | 9 / 4 | 140704 | cli:search_tool, find_code, cli:use_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 2 | yes / yes | 94004 / 75242 | 932 / 712 | 6 / 4 | 69085 | cli:search_tool, cli:search_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 3 | yes / yes | 92202 / 60835 | 496 / 375 | 7 / 3 | 31951 | cli:search_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 4 | yes / yes | 131337 / 152410 | 1100 / 1163 | 9 / 11 | 53085 | cli:search_tool, find_code, cli:search_tool, cli:use_tool, cli:grep, cli:grep, cli:read_file, cli:read_file, cli:grep, cli:read_file, cli:read_file, submit_report, cli:use_tool |
| 5 | yes / yes | 111127 / 79689 | 809 / 600 | 6 / 4 | 42806 | cli:search_tool, cli:search_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 6 | yes / yes | 114117 / 66012 | 845 / 581 | 7 / 3 | 60179 | cli:search_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 7 | yes / yes | 140209 / 64005 | 1387 / 562 | 11 / 3 | 75839 | cli:search_tool, find_code, cli:use_tool, submit_report, cli:use_tool |
| 8 | yes / yes | 126210 / 115677 | 793 / 1069 | 7 / 8 | 35960 | cli:search_tool, cli:search_tool, find_code, cli:use_tool, cli:search_tool, cli:use_tool, cli:grep, cli:read_file, submit_report, cli:use_tool |
| 9 | yes / yes | 111905 / 63447 | 1007 / 645 | 8 / 4 | 44575 | cli:search_tool, find_code, cli:search_tool, cli:use_tool, submit_report, cli:use_tool |
| 10 | yes / yes | 98258 / 64022 | 1081 / 876 | 7 / 4 | 63382 | cli:search_tool, find_code, cli:use_tool, cli:search_tool, submit_report, cli:use_tool |

Right: 10/10 without, 10/10 with find_code. Agent input tokens: 1172533 without, 835698 with (-28.7 %). Jev: 617566 input tokens (≈ $0.0259).

Answers:
1. expected `sansio/app.py`
   without: Flask.full_dispatch_request / handle_user_exception calls App._find_error_handler (src/flask/sansio/app.py) with the exception and request.blueprints. That meth
   with:    src/flask/sansio/app.py — App._find_error_handler (called from Flask.handle_user_exception, Flask.handle_http_exception, and Flask.handle_exception in src/flask
2. expected `sessions.py`
   without: File: src/flask/sessions.py, function SecureCookieSessionInterface.open_session.  On each request, open_session builds a URLSafeTimedSerializer from app.secret_
   with:    In src/flask/sessions.py, SecureCookieSessionInterface.open_session reads the cookie back: it gets the signing serializer, looks up request.cookies.get(self.get
3. expected `config.py`
   without: src/flask/config.py: Config.from_prefixed_env loads environment variables that share a prefix (default FLASK_), strips the prefix, and parses each value with js
   with:    src/flask/config.py — Config.from_prefixed_env. It loads environment variables that start with a prefix (default FLASK_), strips the prefix, and parses each val
4. expected `cli.py`
   without: In `src/flask/cli.py`, the `flask run` command is `run_command`. The `--reload/--no-reload` and `--debugger/--no-debugger` options both default to `None`. After
   with:    The `flask run` command is `run_command` in `src/flask/cli.py`. `--reload/--no-reload` and `--debugger/--no-debugger` both default to `None`. After the app load
5. expected `cli.py`
   without: src/flask/cli.py: find_best_app. locate_app imports the module and, when app_name is None, calls find_best_app. That function looks up the attributes app and ap
   with:    src/flask/cli.py: find_best_app. When the import path has no app name, locate_app imports the module and calls find_best_app. That function checks attributes na
6. expected `helpers.py`
   without: src/flask/helpers.py::_prepare_send_file_kwargs — when send_file (and send_from_directory) builds a response, this helper fills the cache timeout by setting max
   with:    src/flask/helpers.py: `_prepare_send_file_kwargs`. It sets `max_age` from `current_app.get_send_file_max_age` when unset (the former cache timeout) and fills `e
7. expected `sansio/app.py`
   without: src/flask/sansio/app.py: Flask.inject_url_defaults. App and blueprint @url_defaults callbacks are stored on url_default_functions (blueprint ones merged in Blue
   with:    src/flask/sansio/app.py: App.inject_url_defaults. Flask.url_for calls it (src/flask/app.py) immediately before url_adapter.build. The method walks url_default_f
8. expected `json/provider.py`
   without: src/flask/json/provider.py function `_default` (bound as `DefaultJSONProvider.default` and passed to `json.dumps`). `datetime.date` and `datetime.datetime` (dat
   with:    `DefaultJSONProvider` in `src/flask/json/provider.py` uses Python's `json` module and sets `default` to `_default` (`src/flask/json/provider.py`). That function
9. expected `sansio/blueprints.py`
   without: src/flask/sansio/blueprints.py — Blueprint.register. Nested blueprints are only stored by register_blueprint. When the parent is registered, it sets the child’s
   with:    In `Blueprint.register` (`src/flask/sansio/blueprints.py`), the dotted name is `f"{name_prefix}.{self_name}".lstrip(".")`. `self_name` is the `name` registratio
10. expected `testing.py`
   without: A test changes the session before a request with FlaskClient.session_transaction (src/flask/testing.py). Used as `with client.session_transaction() as session:`
   with:    A test changes the session with FlaskClient.session_transaction in src/flask/testing.py. Inside `with client.session_transaction() as sess:`, the method builds 
