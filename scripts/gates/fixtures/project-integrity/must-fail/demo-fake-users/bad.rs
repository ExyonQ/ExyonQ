fn serve(demo_mode: bool) -> Vec<User> {
    if demo_mode {
        return fake_users();
    }
    load_users()
}
