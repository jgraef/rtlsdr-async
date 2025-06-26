use crate::database::Database;

#[derive(Clone, Debug)]
pub struct App {
    pub database: Database,
}
