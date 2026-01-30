import { Dashboard } from "./pages/Dashboard";

const styles = {
  container: {
    maxWidth: 1400,
    margin: "0 auto",
    padding: 24,
    backgroundColor: "#0a0a0a",
    minHeight: "100vh",
    color: "#fff",
  },
};

export function App() {
  return (
    <div style={styles.container}>
      <Dashboard />
    </div>
  );
}
