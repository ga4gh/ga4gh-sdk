from GA4GH import Configuration, TES


config = Configuration("http://localhost:8000/ga4gh/tes/v1")
tes = TES(config)

task = tes.create("../examples/tes-hello.json")
print(f"Created task {task.id}: {task.status()}")

print("Tasks:")
for listed_task in tes.list().tasks:
    print(f"{listed_task.id}: {listed_task.status}")
