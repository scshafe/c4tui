//! Concrete `NavItem` types and the `NavTarget` enum every NavPicker spawned
//! by c4tui produces.
//!
//! This file is the skeleton introduced by Phase 3 Task 1 Step 1.1. The
//! `ViewNavItem`, `ChildViewNavItem`, and `ConnectionNavItem` structs are
//! intentionally empty for this commit; the `impl NavItem` blocks and the
//! concrete fields land in Step 1.4 alongside the trait + tests.

#![allow(dead_code)]

use crate::ids::ViewId;
use crate::view::ConnectionNavigationCandidate;

/// What the user just chose from a navigation picker.
///
/// One union for every NavPicker variant c4tui spawns. The downstream
/// callback (`ModalSlot::on_select`, introduced in Task 5) matches on this
/// variant to produce a `Command`.
///
/// Phase 5 will add a `Link(LinkCandidate)` variant for connection-link
/// navigation; the spawn-site closure is the only seam that needs to learn
/// about the new variant.
#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    /// A view selected from the top-level view picker (clears breadcrumbs).
    View(ViewId),
    /// A view selected from the child-view picker spawned on multi-child drill
    /// (pushes a breadcrumb).
    ChildView(ViewId),
    /// A connection candidate selected from the connection picker (pushes a
    /// breadcrumb and pins the connected element).
    Connection(ConnectionNavigationCandidate),
    // Phase 5 will add:
    //   Link(LinkCandidate),
}

/// Item that yields `NavTarget::View(...)` when selected.
///
/// Fields and `impl NavItem` arrive in Step 1.4.
#[derive(Debug, Clone)]
pub struct ViewNavItem {/* fields per Step 1.4 */}

/// Item that yields `NavTarget::ChildView(...)` when selected. Identical
/// fields to `ViewNavItem` -- the only thing that differs is what the
/// spawn-site does with the resulting NavTarget variant. Keeping them as
/// separate types means the spawn-site's `on_select` closure receives a
/// typed NavTarget and the type-level discipline catches "I forgot to
/// switch which variant I'm building" errors at compile time.
///
/// Fields and `impl NavItem` arrive in Step 1.4.
#[derive(Debug, Clone)]
pub struct ChildViewNavItem {/* fields per Step 1.4 */}

/// Item that yields `NavTarget::Connection(...)` when selected.
///
/// Fields and `impl NavItem` arrive in Step 1.4.
#[derive(Debug, Clone)]
pub struct ConnectionNavItem {/* fields per Step 1.4 */}
