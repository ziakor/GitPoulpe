use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Github,
    Gitlab,
}

impl Provider {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitlab => "GitLab",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub provider: Provider,
    pub api: String,
    pub web: String,
    pub login: String,
    pub scopes: Vec<String>,
}

impl Connection {
    pub fn secret_key(&self) -> String {
        format!("{}:{}", self.provider.caption().to_lowercase(), self.id)
    }

    pub fn host_caption(&self) -> String {
        self.web
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_owned()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionBook {
    pub connections: Vec<Connection>,
}

impl ConnectionBook {
    pub fn upsert(&mut self, connection: Connection) {
        match self.connections.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection,
            None => self.connections.push(connection),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Connection> {
        let at = self.connections.iter().position(|c| c.id == id)?;
        Some(self.connections.remove(at))
    }

    pub fn get(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }
}
