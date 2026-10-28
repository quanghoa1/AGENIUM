## find_code A/B: claude claude-haiku-5-5 on flask (usual read tools vs + find_code)

| # | right (without / with) | tokens in (without / with) | out (without / with) | steps (without / with) | Jev tokens | tools used with find_code |
|---|---|---|---|---|---|---|
| 1 | yes / yes | 52427 / 56346 | 1607 / 1465 | 5 / 3 | 93843 | cli:ToolSearch, find_code, submit_report |
| 2 | yes / yes | 52427 / 52103 | 1766 / 1357 | 4 / 4 | 69085 | cli:ToolSearch, cli:ToolSearch, find_code, submit_report |
| 3 | yes / yes | 66138 / 52762 | 980 / 779 | 4 / 3 | 38773 | cli:ToolSearch, find_code, submit_report |
| 4 | yes / yes | 71159 / 90726 | 2030 / 1919 | 6 / 6 | 54847 | cli:ToolSearch, cli:ToolSearch, find_code, cli:Grep, cli:Read, submit_report |
| 5 | yes / yes | 80673 / 67849 | 1669 / 1322 | 6 / 4 | 42806 | cli:ToolSearch, cli:ToolSearch, find_code, submit_report |
| 6 | yes / yes | 74725 / 55597 | 1925 / 1867 | 5 / 4 | 0 | cli:ToolSearch, cli:Grep, cli:Read, submit_report |
| 7 | yes / yes | 69769 / 112084 | 1835 / 1903 | 7 / 7 | 117390 | cli:ToolSearch, find_code, find_code, cli:ToolSearch, cli:ToolSearch, find_code, submit_report |
| 8 | yes / yes | 53833 / 52478 | 1734 / 1646 | 4 / 4 | 0 | cli:ToolSearch, cli:Grep, cli:Read, submit_report |
| 9 | yes / yes | 88541 / 52545 | 1992 / 1119 | 7 / 3 | 44997 | cli:ToolSearch, find_code, submit_report |
| 10 | yes / yes | 67157 / 52399 | 1789 / 1363 | 4 / 3 | 63382 | cli:ToolSearch, find_code, submit_report |

Right: 10/10 without, 10/10 with find_code. Agent input tokens: 676849 without, 644889 with (-4.7 %). Jev: 525123 input tokens (≈ $0.0221).

Answers:
1. expected `sansio/app.py`
   without: Flask selects the handler in `Flask._find_error_handler` at `src/flask/sansio/app.py:823` (inherited by `Flask` in `src/flask/app.py`; it is called from `handle
   with:    The selection happens in `App._find_error_handler` in `src/flask/sansio/app.py:823-846`.  How it chooses: 1. It gets the exception class and its HTTP code via `
2. expected `sessions.py`
   without: The session cookie is read in `SecureCookieSessionInterface.open_session` in `src/flask/sessions.py:336`.  How it's read back: 1. `get_signing_serializer(app)` 
   with:    The session cookie is read back in `SecureCookieSessionInterface.open_session` in `src/flask/sessions.py:336-348`.  1. It builds the signing serializer with `se
3. expected `config.py`
   without: The function is `Config.from_prefixed_env` in `src/flask/config.py:126`. It takes `prefix` (default `"FLASK"`) and a keyword-only `loads` callable (default `jso
   with:    The function is `Config.from_prefixed_env` in `src/flask/config.py:126-185`. It takes `prefix` (default `"FLASK"`, so it matches `FLASK_*`) and a `loads` callab
4. expected `cli.py`
   without: The decision lives in `run_command` in `src/flask/cli.py` (around lines 904-1000, the `flask run` command).  How it's decided:  1. **Debug mode is the base valu
   with:    The `flask run` command decides via `run_command` in `src/flask/cli.py:940-999`.  - `--reload/--no-reload` and `--debugger/--no-debugger` are click options with
5. expected `cli.py`
   without: When the module is given without an app name (e.g. `flask --app hello`, no `:name`), `locate_app()` in `src/flask/cli.py:241` imports the module and, since `app
   with:    When no app name is given (e.g. `flask --app hello` with no `:name`), Flask finds the app in the imported module through `find_best_app` in `src/flask/cli.py:41
6. expected `helpers.py`
   without: The cache options for `send_file` are filled in by a Flask wrapper around Werkzeug's implementation, in `src/flask/helpers.py`:  1. `send_file()` (helpers.py:40
   with:    The options are filled in by a wrapper in `src/flask/helpers.py`, not by Werkzeug itself.  - `send_file()` (`src/flask/helpers.py:400`) takes the public options
7. expected `sansio/app.py`
   without: URL defaults are stored in `Scaffold.url_default_functions` (a dict keyed by blueprint name, with `None` for app-wide) in `src/flask/sansio/scaffold.py` (~line 
   with:    URL default functions are registered and applied in these places:  **Registration** - `src/flask/sansio/scaffold.py:584` `Scaffold.url_defaults` appends the fun
8. expected `json/provider.py`
   without: The default JSON provider is `DefaultJSONProvider` in `src/flask/json/provider.py`. Its `dumps()` method (line 166) calls `json.dumps` with `default=self.defaul
   with:    The default JSON provider is `DefaultJSONProvider` in `src/flask/json/provider.py`. It hands `json.dumps` a `default=` hook, the module-level function `_default
9. expected `sansio/blueprints.py`
   without: A nested blueprint's dotted name is built when the parent is registered, not when `register_blueprint` is called.  1. `Blueprint.register_blueprint` (src/flask/
   with:    The dotted name is built in `Blueprint.register` in `src/flask/sansio/blueprints.py` (lines 302-304, 376-377):  - `name_prefix` comes from the `name_prefix` opt
10. expected `testing.py`
   without: A test changes the session through `FlaskClient.session_transaction()` in `src/flask/testing.py:135`, a `@contextmanager`. Usage is `with client.session_transac
   with:    A test uses `FlaskClient.session_transaction()` in `src/flask/testing.py:134-182`, as a context manager.  **Changing the session before a request:** - Inside `w
