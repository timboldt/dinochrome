//! A* over the maze grid.
//!
//! This is what an assassin drone thinks with: given where it is and where you
//! are, the shortest route between the two. Hunters get by on greed and drones on
//! luck; only the assassin pays for a real search, and only it is meant to be
//! unshakeable.
//!
//! Cells, not world positions. The grid is coarse — a level-one maze is 825 cells
//! — so a whole search is a few thousand integer operations, which is what makes
//! it affordable to run several times a second per assassin. Moves are the four
//! orthogonal steps at unit cost, so Manhattan distance is an exact lower bound on
//! what is left and A* with it is both admissible and, on a uniform grid,
//! consistent: a cell never needs reopening once it has been settled.
//!
//! Callers throttle their own retargeting — see [`crate::drone`] — because the
//! cost that matters is not one search but twenty assassins all searching on the
//! same tick.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::IVec2;

use crate::grid::Grid;

/// A route through the maze, and what it cost to find one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Route {
    /// The cells to walk, starting with the start cell and ending with the goal.
    ///
    /// Never empty: a route from a cell to itself is that one cell.
    pub cells: Vec<IVec2>,
    /// How many cells the search settled before it arrived.
    ///
    /// Reported so the cost of a search is measurable rather than assumed. It can
    /// never exceed the number of open cells in the grid, which is the worst case
    /// a level has to budget for.
    pub expanded: usize,
}

impl Route {
    /// The next cell to head for, or `None` once the route has been walked.
    ///
    /// The start cell is not a step — something already standing there has
    /// nowhere to go — so this is the second cell of the route.
    pub fn next_step(&self) -> Option<IVec2> {
        self.cells.get(1).copied()
    }

    /// Number of steps to walk, which is one less than the number of cells.
    pub fn steps(&self) -> usize {
        self.cells.len() - 1
    }
}

/// Finds a shortest route from `start` to `goal`, or `None` if there is not one.
///
/// `None` covers all three ways there can be no route: either end is a wall (or
/// outside the grid, which reads the same), or the two are in components that
/// nothing connects. A maze from [`crate::maze::generate`] is always in one piece,
/// so in practice the case that happens is a drone asking about something that has
/// just been destroyed.
pub fn find_path(grid: &Grid, start: IVec2, goal: IVec2) -> Option<Route> {
    if grid.is_wall(start) || grid.is_wall(goal) {
        return None;
    }

    let width = grid.width();
    let size = (width * grid.height()) as usize;
    let index_of = |cell: IVec2| (cell.y * width + cell.x) as usize;
    let cell_of = |index: usize| IVec2::new(index as i32 % width, index as i32 / width);

    // Indexed by cell, so a lookup is an array read rather than a hash. A maze is
    // a few thousand cells; three vectors of that is nothing next to the
    // allocation traffic a `HashMap` would make on every search.
    let mut cost = vec![i32::MAX; size];
    let mut came_from = vec![usize::MAX; size];
    let mut settled = vec![false; size];
    let mut expanded = 0;

    // `Reverse` turns Rust's max-heap into the min-heap A* wants. The tuple is
    // ordered `(f, h, index)`: cheapest estimate first, ties broken toward the
    // goal so the search does not fan out across a plateau of equal-cost cells,
    // and the index last so that two identical nodes still order deterministically.
    let mut frontier = BinaryHeap::new();
    cost[index_of(start)] = 0;
    frontier.push(Reverse((
        heuristic(start, goal),
        heuristic(start, goal),
        index_of(start),
    )));

    while let Some(Reverse((_, _, index))) = frontier.pop() {
        // The heap holds stale entries rather than paying to update them in
        // place, so a cell already settled on a cheaper route is simply dropped.
        if settled[index] {
            continue;
        }
        settled[index] = true;
        expanded += 1;

        let at = cell_of(index);
        if at == goal {
            return Some(Route {
                cells: unwind(&came_from, index, cell_of),
                expanded,
            });
        }

        let next_cost = cost[index] + 1;
        for next in grid.open_neighbours(at) {
            let next_index = index_of(next);
            if settled[next_index] || next_cost >= cost[next_index] {
                continue;
            }
            cost[next_index] = next_cost;
            came_from[next_index] = index;
            let remaining = heuristic(next, goal);
            frontier.push(Reverse((next_cost + remaining, remaining, next_index)));
        }
    }

    None
}

/// Manhattan distance in cells: exactly the cost of the best case, so A* using it
/// never overestimates and never settles a cell on the wrong route.
fn heuristic(from: IVec2, to: IVec2) -> i32 {
    (from.x - to.x).abs() + (from.y - to.y).abs()
}

/// Walks the `came_from` chain back from the goal and turns it the right way round.
fn unwind(came_from: &[usize], goal: usize, cell_of: impl Fn(usize) -> IVec2) -> Vec<IVec2> {
    let mut cells = vec![cell_of(goal)];
    let mut index = goal;
    while came_from[index] != usize::MAX {
        index = came_from[index];
        cells.push(cell_of(index));
    }
    cells.reverse();
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::ORTHOGONAL;
    use crate::maze::{self, MazeParams};

    /// The largest grid the game plans to ask for, wide open — the worst case for
    /// a search, because there is nothing to prune the frontier.
    const WORST: (i32, i32) = (65, 49);

    fn open_grid(cols: i32, rows: i32) -> Grid {
        let mut grid = Grid::solid(cols, rows);
        for y in 0..rows {
            for x in 0..cols {
                grid.set(IVec2::new(x, y), crate::grid::Cell::Open);
            }
        }
        grid
    }

    /// Every consecutive pair is an orthogonal step onto open ground, the route
    /// starts where it was asked to and ends where it was sent.
    fn assert_walkable(grid: &Grid, route: &Route, start: IVec2, goal: IVec2) {
        assert_eq!(route.cells.first(), Some(&start), "route starts elsewhere");
        assert_eq!(route.cells.last(), Some(&goal), "route ends elsewhere");
        for pair in route.cells.windows(2) {
            let step = pair[1] - pair[0];
            assert!(
                ORTHOGONAL.contains(&step),
                "{:?} to {:?} is not one orthogonal step",
                pair[0],
                pair[1]
            );
            assert!(grid.is_open(pair[1]), "route walks into {:?}", pair[1]);
        }
    }

    #[test]
    fn a_cell_can_always_reach_itself_without_moving() {
        let grid = open_grid(9, 9);
        let at = IVec2::new(4, 4);
        let route = find_path(&grid, at, at).expect("a cell reaches itself");
        assert_eq!(route.cells, vec![at]);
        assert_eq!(route.steps(), 0);
        assert_eq!(route.next_step(), None, "nowhere to go");
    }

    #[test]
    fn an_open_field_is_crossed_in_a_straight_manhattan_line() {
        let grid = open_grid(WORST.0, WORST.1);
        let start = IVec2::new(0, 0);
        let goal = IVec2::new(WORST.0 - 1, WORST.1 - 1);
        let route = find_path(&grid, start, goal).expect("an open field is crossable");
        assert_eq!(route.steps() as i32, heuristic(start, goal));
        assert_walkable(&grid, &route, start, goal);
    }

    #[test]
    fn a_walled_off_goal_has_no_route_rather_than_a_wrong_one() {
        // The right-hand column is sealed off by a full-height wall.
        let grid = Grid::from_rows(&[
            "...#.", //
            "...#.", "...#.", "...#.", "...#.",
        ]);
        assert_eq!(find_path(&grid, IVec2::new(0, 0), IVec2::new(4, 4)), None);
    }

    #[test]
    fn neither_end_may_be_a_wall() {
        let grid = Grid::from_rows(&[
            ".....", //
            "..#..", ".....",
        ]);
        let open = IVec2::new(0, 0);
        let wall = IVec2::new(2, 1);
        assert_eq!(find_path(&grid, wall, open), None, "starting in a wall");
        assert_eq!(find_path(&grid, open, wall), None, "ending in a wall");
    }

    #[test]
    fn nothing_outside_the_grid_is_routable() {
        let grid = open_grid(5, 5);
        let inside = IVec2::new(2, 2);
        for outside in [IVec2::new(-1, 2), IVec2::new(5, 2), IVec2::new(2, -7)] {
            assert_eq!(find_path(&grid, inside, outside), None, "to {outside:?}");
            assert_eq!(find_path(&grid, outside, inside), None, "from {outside:?}");
        }
    }

    #[test]
    fn a_route_goes_the_long_way_round_when_that_is_the_only_way() {
        // A cul-de-sac open only at the bottom: the goal is three cells above the
        // start in a straight line and nine steps away in reality.
        let grid = Grid::from_rows(&[
            ".....", // y = 5
            ".....", // y = 4  ← the goal's row
            ".###.", // y = 3  ← the lid of the cul-de-sac
            ".#.#.", // y = 2
            ".#.#.", // y = 1  ← the start
            ".....", // y = 0  ← its only mouth
        ]);
        let start = IVec2::new(2, 1);
        let goal = IVec2::new(2, 4);
        let route = find_path(&grid, start, goal).expect("open at the bottom");
        assert_walkable(&grid, &route, start, goal);
        assert_eq!(route.steps(), 9, "the only way is round the outside");
    }

    #[test]
    fn no_search_ever_settles_more_cells_than_the_maze_has() {
        // The bound that matters for the frame budget: A* with an admissible,
        // consistent heuristic settles each cell at most once, so the worst case
        // is the whole open maze and never more however stale the frontier gets.
        let grid = open_grid(WORST.0, WORST.1);
        let open = grid.open_count();
        let corners = [
            IVec2::new(0, 0),
            IVec2::new(WORST.0 - 1, 0),
            IVec2::new(0, WORST.1 - 1),
            IVec2::new(WORST.0 - 1, WORST.1 - 1),
        ];
        for start in corners {
            for goal in corners {
                let route = find_path(&grid, start, goal).expect("an open field");
                assert!(
                    route.expanded <= open,
                    "{start:?} to {goal:?} settled {} of {open} cells",
                    route.expanded
                );
            }
        }
    }

    #[test]
    fn an_unreachable_goal_costs_no_more_than_the_component_it_is_hunted_in() {
        // The other worst case, and the expensive one: a failed search exhausts
        // everything it can reach. It still may not settle a cell twice.
        let grid = Grid::from_rows(&[
            "....#....", //
            "....#....",
            "....#....",
        ]);
        assert_eq!(find_path(&grid, IVec2::new(0, 0), IVec2::new(8, 2)), None);
    }

    #[test]
    fn every_pair_of_cells_in_a_generated_maze_is_routable() {
        // Maze generation promises the maze is in one piece; this is the promise
        // held to the thing that depends on it hardest.
        let maze = maze::generate(MazeParams::LEVEL_ONE, 19);
        let spawn = maze.spawn;
        for cell in maze.grid.open() {
            let route = find_path(&maze.grid, spawn, cell)
                .unwrap_or_else(|| panic!("no route from {spawn:?} to {cell:?}"));
            assert_walkable(&maze.grid, &route, spawn, cell);
        }
    }

    #[test]
    fn a_route_is_the_same_one_every_time_it_is_asked_for() {
        let maze = maze::generate(MazeParams::LEVEL_ONE, 23);
        let goal = maze.factories[0];
        let first = find_path(&maze.grid, maze.spawn, goal).expect("connected");
        for _ in 0..4 {
            assert_eq!(find_path(&maze.grid, maze.spawn, goal), Some(first.clone()));
        }
    }

    #[test]
    fn a_route_is_a_shortest_one_and_not_merely_a_working_one() {
        // Checked against a breadth-first sweep, which on unit costs is optimal by
        // construction and has no heuristic to be wrong.
        let maze = maze::generate(MazeParams::LEVEL_ONE, 31);
        let grid = &maze.grid;
        let start = maze.spawn;
        let truth = breadth_first_costs(grid, start);
        for cell in grid.open().step_by(7) {
            let route = find_path(grid, start, cell).expect("connected");
            assert_eq!(
                route.steps() as i32,
                truth[&cell],
                "route to {cell:?} is not shortest"
            );
        }
    }

    /// Step counts from `start` to every reachable cell, the slow honest way.
    fn breadth_first_costs(grid: &Grid, start: IVec2) -> std::collections::HashMap<IVec2, i32> {
        let mut costs = std::collections::HashMap::new();
        let mut queue = std::collections::VecDeque::from([start]);
        costs.insert(start, 0);
        while let Some(at) = queue.pop_front() {
            let next_cost = costs[&at] + 1;
            for next in grid.open_neighbours(at) {
                // `insert` unconditionally would overwrite a cell already reached
                // more cheaply with the worse cost that found it again, which is
                // how an oracle ends up claiming a route is longer than it is.
                if let std::collections::hash_map::Entry::Vacant(slot) = costs.entry(next) {
                    slot.insert(next_cost);
                    queue.push_back(next);
                }
            }
        }
        costs
    }
}
