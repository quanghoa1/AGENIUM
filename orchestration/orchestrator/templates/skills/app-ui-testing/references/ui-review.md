# UI review: what to look at, and the report

## Function (does it work)
- The action does what its label says, and the screen shows the result without a manual refresh.
- Errors are explained in words the user understands, next to what caused them, with a way out.
- Input is checked before it is sent; a wrong value keeps what the user typed.
- A slow action shows that it is working; a second click does not run it twice.
- Nothing in the console: an error there is a finding even when the screen looks right.

## Clarity (can a new user tell what to do)
- Each screen has one obvious main action; its title says where the user is.
- Labels name things the way users do, not the code's names (no raw enum values, ids or keys).
- Numbers carry their unit; dates and times are in the user's time zone; lists say what they are sorted by.
- Empty states say why they are empty and what to do next.

## Consistency
- The same thing has the same name, colour and icon on every screen.
- Buttons, spacing, fonts and alignment follow one pattern; the same colour always means the same state.
- Every text is in the chosen language (no leftovers in another one); plurals and dates follow the language.

## Layout
- Nothing overlaps, is cut off or scrolls sideways at the sizes in scope (check a full window and a small one
  with `size`).
- Long names, long lists and many items still fit; the important content is above the fold.
- Contrast is enough to read in light and dark themes.

## Severity
- **Blocker**: a function in scope does not work or loses data.
- **Major**: the user is likely to fail or misunderstand without help.
- **Minor**: friction or inconsistency the user can work around.
- **Polish**: cosmetic.

## Report format

### Functions
| # | Function | Steps | Expected | Actual | Result | Screenshot |
|---|----------|-------|----------|--------|--------|------------|
| 1 | Create a project | Projects > New > name "demo" > Create | The project opens, listed in Projects | ... | pass / fail / not tested | deliverables/ui-review/01-project.png |

### Findings
| # | Severity | Screen | What is wrong | Why it matters | Fix |
|---|----------|--------|---------------|----------------|-----|

### Overall
Two or three sentences: is the app clear and easy to use, and which findings to fix first.
