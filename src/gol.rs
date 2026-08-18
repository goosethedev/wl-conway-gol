#[derive(Debug)]
pub struct GameOfLife {
    grid: Vec<Cell>,
    last: Vec<Cell>,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Alive,
    Dead,
}

impl Cell {
    pub fn is_alive(&self) -> bool {
        *self == Cell::Alive
    }
}

impl GameOfLife {
    pub fn new(width: usize, height: usize) -> Self {
        let grid = vec![Cell::Dead; width * height];
        let last = grid.clone();
        GameOfLife { grid, last, width, height }
    }

    pub fn at(&self, x: usize, y: usize) -> Option<Cell> {
        self.grid.get(y * self.width + x).copied()
    }

    pub fn set_alive(&mut self, x: usize, y: usize) {
        self.set(x, y, Cell::Alive);
    }

    pub fn set_dead(&mut self, x: usize, y: usize) {
        self.set(x, y, Cell::Dead);
    }

    fn set(&mut self, x: usize, y: usize, val: Cell) {
        if let Some(p) = self.grid.get_mut(y * self.width + x) {
            *p = val;
        } else {
            eprintln!("Invalid set op at {x}x{y} on grid {}x{}", self.width, self.height);
        }
    }

    pub fn get_width(&self) -> usize {
        self.width
    }

    pub fn clear(&mut self) {
        self.grid.fill(Cell::Dead);
    }

    pub fn step(&mut self) {
        // Save the current grid
        std::mem::swap(&mut self.grid, &mut self.last);

        let (cells_w, cells_h) = (self.width as isize, self.height as isize);

        for y in 0..cells_h {
            for x in 0..cells_w {
                let mut alive_neighbors = 0;

                for dy in [-1isize, 0, 1] {
                    for dx in [-1isize, 0, 1] {
                        if dx == 0 && dy == 0 {
                            continue;
                        }

                        // Toroidal space (wraps on borders)
                        let i = (x + dx + cells_w) % cells_w;
                        let j = (y + dy + cells_h) % cells_h;

                        if self.last[(i + j * cells_w) as usize].is_alive() {
                            alive_neighbors += 1;
                        }
                    }
                }

                let idx = (x + y * cells_w) as usize;
                let condition = match self.last[idx] {
                    Cell::Alive => (2..=3).contains(&alive_neighbors),
                    Cell::Dead => alive_neighbors == 3,
                };

                self.grid[idx] = if condition { Cell::Alive } else { Cell::Dead };
            }
        }
    }

    pub fn resize(&mut self, new_width: usize, new_height: usize) {
        if new_width == self.width && new_height == self.height {
            return;
        };

        // Swap and resize 'grid' to recalculate from 'last'
        std::mem::swap(&mut self.grid, &mut self.last);
        self.grid.fill(Cell::Dead);
        self.grid.resize(new_width * new_height, Cell::Dead);

        let (cells_w, cells_h) = (self.width.min(new_width), self.height.min(new_height));

        for y in 0..cells_h {
            for x in 0..cells_w {
                self.grid[y * new_width + x] = self.last[y * self.width + x];
            }
        }

        self.width = new_width;
        self.height = new_height;
        self.last.resize(new_width * new_height, Cell::Dead);
    }

    #[allow(clippy::identity_op)]
    pub fn spawn_random_glider(&mut self, x: usize, y: usize) {
        let rand = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        match rand % 4 {
            // Glider SW
            0 => {
                self.set_alive(x + 0, y + 0);
                self.set_alive(x + 1, y + 1);
                self.set_alive(x + 2, y + 1);
                self.set_alive(x + 0, y + 2);
                self.set_alive(x + 1, y + 2);
            }
            // Glider SE
            1 => {
                self.set_alive(x + 2, y + 0);
                self.set_alive(x + 1, y + 1);
                self.set_alive(x + 1, y + 2);
                self.set_alive(x + 0, y + 0);
                self.set_alive(x + 0, y + 1);
            }
            // Glider NE
            2 => {
                self.set_alive(x + 0, y + 1);
                self.set_alive(x + 1, y + 0);
                self.set_alive(x + 1, y + 1);
                self.set_alive(x + 2, y + 0);
                self.set_alive(x + 2, y + 2);
            }
            // Glider NW
            3 => {
                self.set_alive(x + 0, y + 2);
                self.set_alive(x + 1, y + 0);
                self.set_alive(x + 1, y + 1);
                self.set_alive(x + 2, y + 1);
                self.set_alive(x + 2, y + 2);
            }
            _ => {}
        }
    }
}
