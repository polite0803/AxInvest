import { Admin } from "react-admin";
import { authProvider } from "./authProvider";
import { Layout } from "./Layout";

export const App = () => (
  <Admin
    layout={Layout}
    authProvider={authProvider}
  >
  </Admin>
);
