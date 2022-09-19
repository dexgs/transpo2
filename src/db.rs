use diesel::prelude::*;
use diesel_migrations::*;
use chrono::{NaiveDateTime, Local};
use std::path::Path;


pub enum DbConnection {
    #[cfg(feature = "mysql")]
    Mysql(MysqlConnection),
    #[cfg(feature = "postgres")]
    Pg(PgConnection),
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection)
}

#[derive(Clone, Copy)]
pub enum DbBackend {
    #[cfg(feature = "mysql")]
    Mysql,
    #[cfg(feature = "postgres")]
    Pg,
    #[cfg(feature = "sqlite")]
    Sqlite
}

#[derive(Debug)]
#[derive(Queryable)]
#[derive(Insertable)]
#[table_name="uploads"]
pub struct Upload {
    pub id: i64,
    pub file_name: String,
    pub mime_type: String,
    pub password_hash: Option<Vec<u8>>,
    pub remaining_downloads: Option<i32>,
    pub num_accessors: i32,
    pub expire_after: NaiveDateTime
}

table! {
    uploads (id) {
        id -> BigInt,
        file_name -> Text,
        mime_type -> Text,
        password_hash -> Nullable<Binary>,
        remaining_downloads -> Nullable<Integer>,
        num_accessors -> Integer,
        expire_after -> Timestamp,
    }
}

impl Upload {
    // Insert into DB, return number of modified rows, or None if there
    // was a problem.
    pub fn insert(&self, db_connection: &DbConnection) -> Option<usize> {
        let insert = diesel::insert_into(uploads::table)
            .values(self);
        
        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => insert.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => insert.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => insert.execute(c),
        }.ok()
    }

    // Return whether or not an Upload has expired, either based on time or
    // by depleting its maximum number of downloads
    pub fn is_expired(&self) -> bool {
        self.is_expired_time() || self.is_expired_downloads()
    }

    // Return whether or not the expiry date for an upload has been reached
    pub fn is_expired_time(&self) -> bool {
        let now = Local::now().naive_local();
        now > self.expire_after
    }

    // Return whether or not the maximum downloads allowed on this upload have
    // have been expended
    pub fn is_expired_downloads(&self) -> bool {
        if let Some(remaining_downloads) = self.remaining_downloads {
            remaining_downloads <= 0
        } else {
            false
        }
    }

    // Return the Upload with the given ID
    pub fn select_with_id(id: i64, db_connection: &DbConnection) -> Option<Self> {
        let select = uploads::table
            .filter(uploads::id.eq(id))
            .limit(1);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => select.load::<Upload>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<Upload>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<Upload>(c)
        }.ok()?.pop()
    }

    // Decrement the number of remaining downloads on the row with the given ID. Return
    // the number of modified rows.
    pub fn decrement_remaining_downloads(id: i64, db_connection: &DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id)
                .and(uploads::remaining_downloads.is_not_null()));
        let update = diesel::update(target)
            .set(uploads::remaining_downloads.eq(uploads::remaining_downloads - 1));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    // Delete the row with the given ID
    pub fn delete_with_id(id: i64, db_connection: &DbConnection) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));
        let delete = diesel::delete(target);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => delete.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => delete.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => delete.execute(c)
        }.ok()
    }

    // Return a list of IDs for expired (time-based) uploads
    pub fn select_expired(db_connection: &DbConnection) -> Option<Vec<i64>> {
        let now = Local::now().naive_local();
        let select = uploads::table
            .filter(uploads::expire_after.lt(now))
            .select(uploads::id);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => select.load::<i64>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i64>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i64>(c)
        }.ok()
    }

    pub fn select_all(db_connection: &DbConnection) -> Option<Vec<i64>> {
        let select = uploads::table.select(uploads::id);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => select.load::<i64>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i64>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i64>(c)
        }.ok()
    }

    // Increment the accessor count
    pub fn access(db_connection: &DbConnection, id: i64) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::id.eq(id));
        let update = diesel::update(target)
            .set(uploads::num_accessors.eq(uploads::num_accessors + 1));

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        }.ok()
    }

    // Decrement the accessor count
    pub fn revoke(db_connection: &DbConnection, id: i64) -> Option<usize> {
        let target = uploads::table
            .filter(uploads::dsl::id.eq(id));
        let update = diesel::update(target)
            .set(uploads::dsl::num_accessors.eq(uploads::dsl::num_accessors - 1));

        let result = match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => update.execute(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => update.execute(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => update.execute(c)
        };

        result.ok()
    }

    pub fn num_accessors(db_connection: &DbConnection, id: i64) -> Option<i32> {
        let select = uploads::table
            .filter(uploads::dsl::id.eq(id))
            .select(uploads::dsl::num_accessors);

        match db_connection {
            #[cfg(feature = "mysql")]
            DbConnection::Mysql(c) => select.load::<i32>(c),

            #[cfg(feature = "postgres")]
            DbConnection::Pg(c) => select.load::<i32>(c),

            #[cfg(feature = "sqlite")]
            DbConnection::Sqlite(c) => select.load::<i32>(c)
        }.ok()?.pop()
    }
}


fn get_migrations<C, P>(db_connection: &C, path: P) -> Vec<Box<dyn Migration + 'static>>
where C: connection::MigrationConnection,
      P: AsRef<Path>
{
    mark_migrations_in_directory(db_connection, path.as_ref())
        .unwrap()
        .into_iter()
        .filter_map(|(m, is_applied)| if is_applied { None } else { Some(m) })
        .collect()
}

pub fn run_migrations<P>(db_connection: &DbConnection, path: P)
where P: AsRef<Path>
{
    let path = path.as_ref();
    let stdout = &mut std::io::stdout();
    match db_connection {
        #[cfg(feature = "mysql")]
        DbConnection::Mysql(c) => {
            let migrations: Vec<_> = get_migrations(c, path.join("migrations"));
            diesel_migrations::run_migrations(c, migrations, stdout)
        },
        #[cfg(feature = "postgres")]
        DbConnection::Pg(c) => {
            let migrations: Vec<_> = get_migrations(c, path.join("pg_migrations"));
            diesel_migrations::run_migrations(c, migrations, stdout)
        },
        #[cfg(feature = "sqlite")]
        DbConnection::Sqlite(c) => {
            let migrations: Vec<_> = get_migrations(c, path.join("migrations"));
            diesel_migrations::run_migrations(c, migrations, stdout)
        }
    }.expect("Running database migrations");
}

pub fn parse_db_backend(db_url: &str) -> Option<DbBackend> {
    if db_url.starts_with("mysql://") {
        #[cfg(feature = "mysql")]
        return Some(DbBackend::Mysql);
    } else if db_url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        return Some(DbBackend::Pg);
    } else {
        #[cfg(feature = "sqlite")]
        return Some(DbBackend::Sqlite);
    }

    #[cfg(not(all(feature = "mysql", feature = "postgres", feature = "sqlite")))]
    None
}

pub fn establish_connection(db_backend: DbBackend, db_url: &str) -> DbConnection {
    match db_backend {
        #[cfg(feature = "mysql")]
        DbBackend::Mysql => DbConnection::Mysql(
            MysqlConnection::establish(&db_url)
            .expect("Establishing MySQL connection")),

            #[cfg(feature = "postgres")]
        DbBackend::Pg => DbConnection::Pg(
            PgConnection::establish(&db_url)
            .expect("Establishing PostgreSQL connection")),

            #[cfg(feature = "sqlite")]
        DbBackend::Sqlite => {
            let connection = SqliteConnection::establish(&db_url)
                .expect("Establishing SQLite connection");
            connection.execute("PRAGMA busy_timeout = 15000;")
                .expect("Setting busy timeout");
            DbConnection::Sqlite(connection)
        }
    }
}

pub type DbConnectionInfo = (DbBackend, String);

pub fn establish_connection_info(db_connection_info: &DbConnectionInfo) -> DbConnection {
    establish_connection(db_connection_info.0, &db_connection_info.1)
}
