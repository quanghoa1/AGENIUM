//! A turn that failed on its login, taken again once (ADR-134). Several runs on one CLI account share its refresh token,
//! which is single-use: a run that refreshes it a moment after another did (before the other's new login reached the
//! user's file, which takes up to `LOGIN_SYNC`) is refused and its turn ends on a login error. The run then waits for
//! the other run to hand its login over, takes the user's current login into its own copy and runs the turn again;
//! a second login error is a login that is really gone.

use super::*;

/// What the CLI is told when its session goes on after the login error.
const GO_ON: &str = "Your last turn stopped on a login error, now fixed. Go on with the task from where you stopped.";

impl CliRun<'_> {
    /// Starts the failed turn again after bringing the user's current login in (`resync`). A session the CLI has
    /// worked in goes on (told to go on); one that did nothing yet starts again with the turn's input `last`.
    pub(super) async fn relogin(
        &self,
        p: &mut Proc,
        env: &BTreeMap<String, String>,
        home: &std::path::Path,
        st: &State,
        last: &str,
        resync: &(dyn Fn() + Sync),
    ) -> Result<Proc, Outcome> {
        p.stop().await;
        tokio::time::sleep(LOGIN_SYNC + Duration::from_secs(1)).await;
        resync();
        match st.session.as_deref().filter(|_| st.steps > 0 || st.resumed) {
            Some(id) => self.start_turn(env, home, Some(id), &self.driver.user_input(GO_ON)),
            None => self.start_turn(env, home, None, last),
        }
    }
}
