#[derive(Debug)]
pub struct GameOfLife {
    pub grid: Vec<bool>,
    last_grid: Option<Vec<bool>>,
    pub cells_w: usize,
    pub cells_h: usize,
}

impl GameOfLife {
    pub fn new(cells_w: usize, cells_h: usize) -> Self {
        let grid = vec![false; cells_w * cells_h];
        GameOfLife { grid, last_grid: None, cells_w, cells_h }
    }

    pub fn step(&mut self) {
        let (cells_w, cells_h) = (self.cells_w as isize, self.cells_h as isize);
        let last_grid = self.grid.clone();

        for y in 0..cells_h {
            for x in 0..cells_w {
                let mut alive_neighbors = 0;

                for dy in [-1isize, 0, 1] {
                    for dx in [-1isize, 0, 1] {
                        let (i, j) = (x + dx, y + dy);
                        if (dx == 0 && dy == 0)
                            || (j < 0 || j >= cells_h)
                            || (i < 0 || i >= cells_w)
                        {
                            continue;
                        }

                        if last_grid[(i + j * cells_w) as usize] {
                            alive_neighbors += 1;
                        }
                    }
                }

                let idx = (x + y * cells_w) as usize;
                self.grid[idx] = match self.grid[idx] {
                    true => (2..=3).contains(&alive_neighbors),
                    false => alive_neighbors == 3,
                }
            }
        }

        self.last_grid = Some(last_grid);
    }
}
